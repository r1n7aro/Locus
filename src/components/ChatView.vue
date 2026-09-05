
<script setup lang="ts">
import { ref, nextTick, watch, computed, onMounted, onUnmounted } from "vue";
import { BookOpen, Copy, Crosshair, ExternalLink, FileDiff, FolderOpen, GitFork, PanelTopOpen, PencilLine, Undo2 } from "lucide";
import {
  selectUnityAsset,
  openUnityAssetInspector,
  selectUnitySceneObject,
  openUnitySceneObjectInspector,
  classifyUnitySceneObjectError,
  openFileExternal,
  showInFolder,
} from "../services/unity";
// undoPreview removed — undo UI moved to ChatChangesPanel
import type { ChatComposerSendPayload, ChatMessage, AgentInfo, TokenUsage, ModelOption, PendingQuestion, PendingToolConfirm, EffortLevel, SessionSummary, AssetDbScanEvent, ScanStats, ImageAttachment, AssetRefAttachment, ManagedLocalFileAttachment, SkillManifest, UserIntentMeta, SessionContextExportRequest, CodexTransportMode, AssistantRenderPart, UnityConnectionStatus, KnowledgeDocumentType, SessionTurnPreview } from "../types";
import type { ChangedFile, ToolCallDisplay } from "../types";
import ModelEffortSelector from "./ModelEffortSelector.vue";
import SessionPanel from "./chat/SessionPanel.vue";
import SessionCompactPicker from "./chat/SessionCompactPicker.vue";
import ChatTranscript from "./chat/ChatTranscript.vue";
import ChatTurnNavigationRail from "./chat/ChatTurnNavigationRail.vue";
import ChatStatusIndicators from "./chat/ChatStatusIndicators.vue";
import QueuedFollowUpImages from "./chat/QueuedFollowUpImages.vue";
import RichChatInput from "./chat/RichChatInput.vue";
import TokenUsageBar from "./chat/TokenUsageBar.vue";
import SessionContextUsageWindow from "./SessionContextUsageWindow.vue";
import AskUserCard from "./chat/AskUserCard.vue";
import ToolConfirmCard from "./chat/ToolConfirmCard.vue";
import ToolConfirmBatchCard from "./chat/ToolConfirmBatchCard.vue";
import FileDiffViewer from "./diff/FileDiffViewer.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseContextMenu from "./ui/BaseContextMenu.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { refetchDiffByKey } from "../services/diff";
import type { WorkspaceRef } from "../services/project";
import { openChatDiffReviewWindow } from "../services/chatDiffReviewWindow";
import { broadcastPlanApprovalResolved, openPlanViewWindow } from "../services/planViewWindow";
import { openContextCompactionWindow } from "../services/contextCompactionWindow";
import {
  openLocusAssetInspectorWorkbenchTab,
  type LocusAssetInspectorTarget,
} from "../services/locusAssetInspector";
import { normalizeAppError } from "../services/errors";
import { knowledgeRevealTarget } from "../services/knowledge";
import { t } from "../i18n";
import { useChatChangesStore } from "../stores/chatChanges";
import { useChatStore } from "../stores/chat";
import { useUiStore } from "../stores/ui";
import { useNotificationStore } from "../stores/notification";
import { useProjectStore } from "../stores/project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import {
  captureScrollAnchor,
  captureLiveScrollAnchor,
  captureSessionScrollState,
  isNearBottom,
  resolveSessionScrollTop,
  restoreLiveScrollAnchor,
  restoreScrollAnchor,
  shouldRestoreBottomFromTopAnchorState,
  type LiveScrollAnchorSnapshot,
  type SessionScrollState,
} from "../composables/chatScrollState";
import {
  createCoalescedScrollScheduler,
  createSettledScrollScheduler,
  createUserScrollIntentTracker,
  hasRunningToolCall,
  shouldAutoScrollToBottom,
  shouldShowWaitingPlaceholder,
} from "../composables/chatViewStability";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../composables/resizeObserver";
import { forwardWheelToElement } from "../composables/chatWheelPassthrough";
import { STREAMING_RENDER_THROTTLE_MS } from "../composables/streamingRenderThrottle";
import type { StreamingTextSource } from "../composables/streamingTextChunks";
import { canOpenInEditor } from "../composables/useHideMeta";
import { useDiffProgress } from "../composables/useDiffProgress";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import {
  createDoublePressShortcutTracker,
  DOUBLE_PRESS_SHORTCUT_INTERVAL_MS,
  formatShortcut,
  matchesShortcut,
  useKeyboardShortcuts,
} from "../composables/useKeyboardShortcuts";
import {
  getChatSubmitModifierLabel,
  useChatInputSettings,
} from "../composables/useChatInputSettings";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { useSessionUndoSettings } from "../composables/useSessionUndoSettings";
import { useKnowledgeDocumentOpen } from "../composables/useKnowledgeDocumentOpen";
import { useKnowledgeAccessMode } from "../composables/useKnowledgeAccessMode";
import {
  buildChatMessageClipboardPayload,
  buildUserMessageDraft,
  writeChatMessageClipboard,
  type UserMessageDraft,
} from "../composables/chatMessageDraft";
import { emptyComposerIntent } from "../composables/chatInputIntents";
import { isMessageFromActiveStream } from "../composables/chatMessageFork";
import { selectionTextAtPoint } from "../composables/chatSelectionContext";
import { logToolCollapseTrace, previewTraceText } from "../services/toolCollapseTrace";
import {
  captureTranscriptLayoutSnapshot,
  recordLayoutDiagnostic,
  traceViewportAnchorSample,
} from "../services/layoutDiagnostics";
import { useInternalDragController } from "../composables/useInternalDrag";
import {
  startWorkbenchReferenceInternalDrag,
} from "./workbench/workbenchReferenceDrag";

type ChatLayoutMode = "auto" | "horizontal" | "vertical";
type ResolvedChatLayoutMode = "horizontal" | "vertical";
type NewChatShortcutAction = "keepCurrent" | "replaceCurrent" | "newTab";
type NewChatRequestSource = "control" | "shortcut";

interface ScopedQueuedFollowUp {
  displayText: string;
  images?: ImageAttachment[];
  canInsert: boolean;
  isInserting: boolean;
}

const chatChangesStore = useChatChangesStore();
const {
  state: sessionUndoSettings,
  load: loadSessionUndoSettings,
} = useSessionUndoSettings();
void loadSessionUndoSettings().catch((error: unknown) => {
  console.warn("[session-undo] failed to load setting for Chat Changes:", error);
});
const chatStore = useChatStore();
const uiStore = useUiStore();
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();
const workspaceContextStore = useWorkspaceContextStore();
const internalDrag = useInternalDragController();
const { state: shortcutState } = useKeyboardShortcuts();
const { state: chatInputSettings } = useChatInputSettings();
const { state: displaySettings } = useDisplaySettings();
const {
  openDocument: openLegacyKnowledgeDocument,
  openInKnowledge: openLegacyKnowledgeDocumentInKnowledge,
} = useKnowledgeDocumentOpen();
const { state: knowledgeAccessState, setMode: setKnowledgeAccessMode } = useKnowledgeAccessMode();

const planModeActive = computed(() => (
  props.scopedSession ? !!props.planModeActive : chatStore.activeSessionPlanMode
));
const isViewingSubagent = computed(() => {
  if (props.scopedSession) return !!props.isViewingSubagent;
  if (!props.activeSessionId) return false;
  const session = props.sessions.find(s => s.id === props.activeSessionId);
  return !!session?.parentSessionId;
});
const activeQueuedFollowUp = computed(() => (
  props.scopedSession ? props.queuedFollowUp ?? null : chatStore.activeQueuedFollowUp
));
const showQueuedFollowUp = computed(() =>
  !!activeQueuedFollowUp.value && props.isStreaming,
);
const showCompactQueued = computed(() =>
  (props.scopedSession ? !!props.compactQueued : chatStore.isCompactQueued)
  && props.isStreaming
  && !isViewingSubagent.value,
);
const diffProgress = useDiffProgress(() => props.workspaceRef ?? null);
const diffProgressWidth = computed(() => `${diffProgress.progress.value * 100}%`);
const chatInputPlaceholder = computed(() => {
  if (chatInputSettings.submitMode === "mod-enter-send") {
    return t("chat.input.placeholderModifierSend", getChatSubmitModifierLabel());
  }
  return t("chat.input.placeholder");
});
const runningSendLabel = computed(() =>
  isViewingSubagent.value || chatInputSettings.runningSendMode === "insert"
    ? t("chat.input.queuedFollowUpInsert")
    : t("chat.input.queue"),
);
const inputControlsCollapsed = ref(false);
const inputControlsSwitching = ref(false);
const INPUT_CONTROLS_SWITCH_VISIBLE_MS = 120;
const inputControlsToggleTitle = computed(() => (
  inputControlsCollapsed.value
    ? t("chat.input.showControls")
    : t("chat.input.hideControls")
));
let inputControlsSwitchTimer: ReturnType<typeof setTimeout> | null = null;

function clearInputControlsSwitchTimer() {
  if (!inputControlsSwitchTimer) return;
  clearTimeout(inputControlsSwitchTimer);
  inputControlsSwitchTimer = null;
}

function toggleInputControlsCollapsed() {
  inputControlsCollapsed.value = !inputControlsCollapsed.value;
  inputControlsSwitching.value = true;
  clearInputControlsSwitchTimer();
  inputControlsSwitchTimer = setTimeout(() => {
    inputControlsSwitching.value = false;
    inputControlsSwitchTimer = null;
  }, INPUT_CONTROLS_SWITCH_VISIBLE_MS);
}

const changesSessionState = computed(() => (
  chatChangesStore.sessionState(props.activeSessionId)
));
const inlineDiffState = computed(() => (
  chatChangesStore.inlineDiffStateForSession(props.activeSessionId)
));
const inlineDiffPayload = computed(() => inlineDiffState.value?.payload ?? null);
const inlineDiffLoading = computed(() => inlineDiffState.value?.loading ?? false);
const inlineDiffError = computed(() => inlineDiffState.value?.error ?? null);
const inlineDiffRequestKey = computed(() => inlineDiffState.value?.requestKey ?? null);
const currentChangesPanelVisible = computed(() => (
  changesSessionState.value?.panelVisible ?? false
));
const showInlineDiff = computed(() => (
  !!inlineDiffPayload.value || inlineDiffLoading.value || !!inlineDiffError.value
));
const hasPanelToggleRow = computed(() => (
  Boolean(props.activeSessionId?.trim())
  && (
    sessionUndoSettings.enabled
    || chatChangesStore.hasChangesForSession(props.activeSessionId)
  )
));

const chatDiffViewerRef = ref<InstanceType<typeof FileDiffViewer> | null>(null);
const chatDiffTabOptions = computed(() => [
  { value: "semantic", label: t("diff.tabs.semantic") },
  { value: "text", label: t("diff.tabs.text") },
]);

async function onChatDiffLfsPulled() {
  const payload = inlineDiffPayload.value;
  if (!payload) return;
  try {
    const updated = await refetchDiffByKey(payload.key);
    if (updated && inlineDiffState.value?.payload?.key === payload.key) {
      inlineDiffState.value.payload = updated;
    }
  } catch (e) {
    console.error("[ChatView] refetch after LFS pull failed:", e);
  }
}

const props = defineProps<{
  messages: ChatMessage[];
  /** Legacy throttled typewriter text. Pass "" (and supply typedTextStream +
   * hasStreamingText) so growing text never transits props; the transcript
   * then renders the typewriter through its incremental stream path. */
  streamingText: string;
  typedTextStream?: StreamingTextSource | null;
  hasStreamingText?: boolean;
  streamingTextOrder?: number;
  isStreaming: boolean;
  isCancelling?: boolean;
  canResumeInterrupted?: boolean;
  isCompacting: boolean;
  isThinking: boolean;
  hasThinking: boolean;
  thinkingOrder?: number;
  thinkingDuration: number;
  liveRenderParts?: AssistantRenderPart[];
  livePartStreams?: ReadonlyMap<string, StreamingTextSource> | null;
  activeToolCalls: ToolCallDisplay[];
  agents: AgentInfo[];
  selectedAgentId: string;
  agentLocked: boolean;
  models: ModelOption[];
  selectedModelId: string;
  codexTransport?: CodexTransportMode;
  effort: EffortLevel;
  effortSupported: boolean;
  effortLevels: EffortLevel[];
  multiAgentEnabled?: boolean;
  fastModeEnabled: boolean;
  fastModeAvailable: boolean;
  tokenUsage: TokenUsage;
  codexConnected?: boolean;
  pendingQuestion: PendingQuestion | null;
  pendingToolConfirms: PendingToolConfirm[];
  sessions: SessionSummary[];
  activeSessionId: string | null;
  pendingSessionId?: string | null;
  unityConnected?: boolean;
  unityPluginStatus?: "missing" | "outdated" | null;
  unityPluginInstalling?: boolean;
  unityLaunching?: boolean;
  unityLaunchState?: "idle" | "starting" | "waitingConnection";
  unityConnectionStatus?: UnityConnectionStatus | null;
  workspaceRef?: WorkspaceRef | null;
  projectId?: string;
  projectServices?: string[];
  workingDir?: string;
  scanPhase?: AssetDbScanEvent | null;
  lastScanStats?: ScanStats | null;
  skills?: SkillManifest[];
  managedLocalFiles?: ManagedLocalFileAttachment[];
  streamingSessionIds?: Set<string>;
  undoableMessageIds?: Set<string>;
  layoutMode?: ChatLayoutMode;
  defaultSessionPanelCollapsed?: boolean;
  sessionPanelStorageScope?: string;
  showSessionNavigation?: boolean;
  contentStartInset?: number;
  /** Uses pane-owned session state while preserving the standard Chat surface. */
  scopedSession?: boolean;
  isViewingSubagent?: boolean;
  sessionSurfaceKey?: string;
  queuedFollowUp?: ScopedQueuedFollowUp | null;
  compactQueued?: boolean;
  planModeActive?: boolean;
  currentRunId?: string | null;
  composerValue?: string;
  composerDraftStateKey?: string;
  referenceDropAvailable?: boolean;
  referenceDropActive?: boolean;
  managedNativeDrops?: boolean;
  shortcutActive?: boolean;
  newChatShortcutAction?: NewChatShortcutAction;
  checkUndoDirty?: (assistantMessageId: string) => Promise<ChangedFile[]>;
  undoConversation?: (targetMessageId: string | null) => Promise<boolean>;
  undoFilesAndConversation?: (
    targetMessageId: string | null,
    assistantMessageId: string,
    acceptDirty: boolean,
  ) => Promise<boolean>;
  restoreComposerDraft?: (draft: UserMessageDraft) => void | Promise<void>;
  forkFromMessage?: (messageId: string) => void | Promise<unknown>;
  exitPlanMode?: () => void | Promise<unknown>;
  sessionHistoryLoading?: boolean;
  sessionHistoryHasMore?: boolean;
  loadOlderHistory?: () => Promise<boolean>;
  sessionUserMessageIds?: string[];
  loadSessionTurnPreview?: (messageId: string) => Promise<SessionTurnPreview | null>;
  loadSessionHistoryThroughMessage?: (messageId: string) => Promise<boolean>;
}>();

const turnNavigationUserMessageIds = computed(() => (
  props.scopedSession ? props.sessionUserMessageIds ?? [] : chatStore.sessionUserMessageIds
));
const turnNavigationLoadPreview = computed(() => (
  props.scopedSession ? props.loadSessionTurnPreview : chatStore.loadSessionTurnPreview
));
const turnNavigationLoadTurn = computed(() => (
  props.scopedSession
    ? props.loadSessionHistoryThroughMessage
    : chatStore.loadSessionHistoryThroughMessage
));

watch(inlineDiffLoading, (loading) => {
  if (loading) diffProgress.reset(inlineDiffRequestKey.value);
});

const chatContentStyle = computed(() => ({
  paddingLeft: `${Math.max(0, props.contentStartInset ?? 0)}px`,
}));

function hasRunningUnityRecompile(calls: ToolCallDisplay[] | undefined): boolean {
  return !!calls?.some((call) =>
    (call.name === "unity_recompile" && call.status === "running")
    || hasRunningUnityRecompile(call.nestedToolCalls),
  );
}

const unityRecompileActive = computed(() => hasRunningUnityRecompile(props.activeToolCalls));

const emit = defineEmits<{
  send: [text: string, images: ImageAttachment[], assetRefs: AssetRefAttachment[], overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null }];
  compact: [];
  fork: [];
  cancel: [];
  resume: [];
  selectAgent: [id: string];
  selectModel: [id: string];
  selectEffort: [level: EffortLevel];
  selectFastMode: [enabled: boolean];
  selectMultiAgent: [enabled: boolean];
  exportSessionContext: [request: SessionContextExportRequest];
  reviewSessionContext: [request: SessionContextExportRequest];
  removeManagedComposerFile: [fileId: string];
  answerQuestion: [answer: string];
  answerToolConfirm: [questionId: string, answer: string];
  answerAllToolConfirms: [questionIds: string[], answer: string];
  insertQueuedFollowUp: [];
  reEditQueuedFollowUp: [];
  deleteQueuedFollowUp: [];
  applyKnowledgeProposal: [proposalId: string];
  ignoreKnowledgeProposal: [proposalId: string];
  updateComposerValue: [value: string];
  openThinking: [content: string];
  requestPlanMode: [active: boolean];
  openKnowledgeDocument: [request: { docType: KnowledgeDocumentType; path: string; workspaceRef: WorkspaceRef }];
  openKnowledgeDocumentInKnowledge: [request: { docType: KnowledgeDocumentType; path: string; workspaceRef: WorkspaceRef }];
  selectSession: [id: string];
  newChat: [request: { source: NewChatRequestSource }];
  renameSession: [id: string, title: string];
  archiveSession: [id: string];
  deleteSession: [id: string];
  startScan: [];
  installPlugin: [];
  launchUnityProject: [];
  layoutModeChange: [mode: ResolvedChatLayoutMode];
  composerFocus: [];
}>();

const lightboxSrc = ref("");
const contextStatsOpen = ref(false);
function openLightbox(src: string) {
  lightboxSrc.value = src;
}

function handleInsertQueuedFollowUp() {
  if (props.scopedSession) {
    emit("insertQueuedFollowUp");
    return;
  }
  void chatStore.insertActiveQueuedFollowUp();
}
function handleDeleteQueuedFollowUp() {
  if (props.scopedSession) {
    emit("deleteQueuedFollowUp");
    return;
  }
  void chatStore.deleteActiveQueuedFollowUp();
}
function handleReEditQueuedFollowUp() {
  if (props.scopedSession) {
    emit("reEditQueuedFollowUp");
    return;
  }
  void chatStore.reEditActiveQueuedFollowUp();
}

function handleApplyKnowledgeProposal(proposalId: string) {
  if (props.scopedSession) {
    emit("applyKnowledgeProposal", proposalId);
    return;
  }
  void chatStore.applyKnowledgeProposal(proposalId);
}

function handleIgnoreKnowledgeProposal(proposalId: string) {
  if (props.scopedSession) {
    emit("ignoreKnowledgeProposal", proposalId);
    return;
  }
  void chatStore.ignoreKnowledgeProposal(proposalId);
}
function closeLightbox() {
  lightboxSrc.value = "";
}

/**
 * Resource actions belong to the Chat surface that rendered the reference.
 * Scoped surfaces never inherit the globally focused checkout: a missing
 * workspace binding disables the action instead of targeting another pane.
 */
function resourceWorkspaceRef(): WorkspaceRef | null {
  if (props.workspaceRef) return props.workspaceRef;
  if (props.scopedSession) return null;
  return projectStore.requireWorkspaceRef();
}

const resourceActionsAvailable = computed(() => (
  !props.scopedSession || !!props.workspaceRef
));

async function openLocusAssetInspector(
  target: LocusAssetInspectorTarget,
): Promise<boolean> {
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return false;
  return openLocusAssetInspectorWorkbenchTab(workspaceRef, target);
}

type AssetRefContextMenuTarget =
  | {
      kind: "asset";
      filePath: string;
      assetPath: string;
    }
  | {
      kind: "file";
      filePath: string;
      entryKind: "file" | "folder";
    }
  | {
      kind: "knowledge";
      docType: KnowledgeDocumentType;
      path: string;
      filePath: string;
    }
  | {
      kind: "sceneObject";
      filePath: string;
      scenePath: string;
      objectPath: string;
    };

type KnowledgeRefContextMenuTarget = Extract<AssetRefContextMenuTarget, { kind: "knowledge" }>;

type AssetRefContextMenuState = {
  x: number;
  y: number;
  target: AssetRefContextMenuTarget;
};

type MessageContextMenuState = {
  x: number;
  y: number;
  messageId: string | null;
  selectionText: string;
};

const KNOWLEDGE_DOCUMENT_ROOT_RE = /^(design|plan|memory|skill|reference)\/(.+\.md)$/i;
const KNOWLEDGE_DOCUMENT_FILE_RE = /^Locus\/knowledge\/(design|plan|memory|skill|reference)\/(.+\.md)$/i;
const assetRefCtxMenu = ref<AssetRefContextMenuState | null>(null);
const messageCtxMenu = ref<MessageContextMenuState | null>(null);

function isUnityEmbeddedWindow() {
  if (typeof window === "undefined") return false;
  return window.location.pathname === "/unity-embed";
}

function isUnityAssetPath(filePath: string) {
  return /^(Assets|Packages)\//.test(filePath.replace(/\\/g, "/"));
}

function shouldSelectUnityAsset(filePath: string) {
  return resourceActionsAvailable.value && isUnityAssetPath(filePath) && props.unityConnected;
}

function shouldOpenUnityAssetInspector(e: MouseEvent, filePath: string) {
  return (e.ctrlKey || e.metaKey)
    && isUnityEmbeddedWindow()
    && isUnityAssetPath(filePath)
    && !canOpenInEditor(filePath);
}

function shouldUseUnitySceneObjectRef(scenePath: string, objectPath: string) {
  return resourceActionsAvailable.value
    && /\.unity$/i.test(scenePath.replace(/\\/g, "/"))
    && objectPath.trim().length > 0
    && props.unityConnected;
}

function shouldOpenUnitySceneObjectInspector(e: MouseEvent, scenePath: string, objectPath: string) {
  return (e.ctrlKey || e.metaKey) && shouldUseUnitySceneObjectRef(scenePath, objectPath);
}

const assetRefContextCanSelectInUnity = computed(() => {
  const target = assetRefCtxMenu.value?.target;
  if (!target) return false;
  if (target.kind === "sceneObject") {
    return shouldUseUnitySceneObjectRef(target.scenePath, target.objectPath);
  }
  if (target.kind !== "asset") return false;
  return shouldSelectUnityAsset(target.assetPath);
});
const knowledgeAccessMode = computed(() => knowledgeAccessState.mode);

const assetRefContextIsKnowledge = computed(() =>
  assetRefCtxMenu.value?.target.kind === "knowledge",
);

const assetRefContextCanOpenInEditor = computed(() => {
  const target = assetRefCtxMenu.value?.target;
  return resourceActionsAvailable.value
    && !!target
    && !(target.kind === "knowledge" || (target.kind === "file" && target.entryKind === "folder"));
});

const assetRefContextCanOpenLocusInspector = computed(() => {
  if (!resourceActionsAvailable.value) return false;
  const target = assetRefCtxMenu.value?.target;
  if (!target) return false;
  if (target.kind === "sceneObject") {
    return shouldUseUnitySceneObjectRef(target.scenePath, target.objectPath);
  }
  return target.kind === "asset";
});

const assetRefContextSupportsUnity = computed(() => {
  const target = assetRefCtxMenu.value?.target;
  return target?.kind === "asset" || target?.kind === "sceneObject";
});

function closeAssetRefContextMenu() {
  assetRefCtxMenu.value = null;
}

function closeMessageContextMenu() {
  messageCtxMenu.value = null;
}

function normalizeAssetRefDatasetPath(path: string | undefined): string {
  return (path ?? "").trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function toKnowledgeDocumentType(value: string): KnowledgeDocumentType | null {
  const normalized = value.trim().toLowerCase();
  if (
    normalized === "design" ||
    normalized === "plan" ||
    normalized === "memory" ||
    normalized === "skill" ||
    normalized === "reference"
  ) {
    return normalized;
  }
  return null;
}

function parseKnowledgeDocumentRefPath(filePath: string): KnowledgeRefContextMenuTarget | null {
  const normalized = normalizeAssetRefDatasetPath(filePath).replace(/^\/+/, "");
  if (!normalized) return null;

  const fileMatch = normalized.match(KNOWLEDGE_DOCUMENT_FILE_RE);
  if (fileMatch) {
    const docType = toKnowledgeDocumentType(fileMatch[1] ?? "");
    const path = normalizeAssetRefDatasetPath(fileMatch[2]);
    if (docType && path) {
      return {
        kind: "knowledge",
        docType,
        path: `${docType}/${path}`,
        filePath: normalized,
      };
    }
  }

  const rootMatch = normalized.match(KNOWLEDGE_DOCUMENT_ROOT_RE);
  if (rootMatch) {
    const docType = toKnowledgeDocumentType(rootMatch[1] ?? "");
    const path = normalizeAssetRefDatasetPath(rootMatch[2]);
    if (docType && path) {
      return {
        kind: "knowledge",
        docType,
        path: `${docType}/${path}`,
        filePath: `Locus/knowledge/${docType}/${path}`,
      };
    }
  }

  return null;
}

function assetContextTargetFromElement(target: Element): AssetRefContextMenuTarget | null {
  const unityObjectIdentity = target.closest(
    ".unity-object-identity[data-unity-ref-kind]",
  ) as HTMLElement | null;
  if (unityObjectIdentity) {
    const refKind = unityObjectIdentity.dataset.unityRefKind;
    const refPath = normalizeAssetRefDatasetPath(unityObjectIdentity.dataset.unityRefPath);
    if ((refKind === "asset" || refKind === "subObject") && refPath && isUnityAssetPath(refPath)) {
      return {
        kind: "asset",
        filePath: refPath,
        assetPath: refPath,
      };
    }
    if (refKind === "sceneObject") {
      const sceneRef = refPath.match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
      const scenePath = normalizeAssetRefDatasetPath(sceneRef?.[1]);
      const objectPath = normalizeAssetRefDatasetPath(sceneRef?.[2]);
      if (scenePath && objectPath) {
        return {
          kind: "sceneObject",
          filePath: scenePath,
          scenePath,
          objectPath,
        };
      }
    }
  }

  const knowledgeRef = target.closest(
    ".md-knowledge-ref[data-knowledge-path], .asset-chip[data-ref-kind='knowledge']",
  ) as HTMLElement | null;
  if (knowledgeRef) {
    const path = normalizeAssetRefDatasetPath(knowledgeRef.dataset.knowledgePath);
    const docType = toKnowledgeDocumentType(knowledgeRef.dataset.knowledgeType ?? "")
      ?? toKnowledgeDocumentType(path.split("/")[0] ?? "");
    if (docType && path) {
      return {
        kind: "knowledge",
        docType,
        path,
        filePath: `Locus/knowledge/${path}`,
      };
    }
  }

  const sceneObjectRef = target.closest(
    ".md-unity-scene-object-ref, .asset-chip[data-ref-kind='sceneObject']",
  ) as HTMLElement | null;
  if (sceneObjectRef) {
    const scenePath = normalizeAssetRefDatasetPath(sceneObjectRef.dataset.scenePath);
    const objectPath = normalizeAssetRefDatasetPath(sceneObjectRef.dataset.sceneObjectPath);
    if (scenePath && objectPath) {
      return {
        kind: "sceneObject",
        filePath: scenePath,
        scenePath,
        objectPath,
      };
    }
  }

  const unityAssetRef = target.closest(
    ".md-unity-asset-ref, .md-file-ref[data-asset-path], .md-asset-chip, .asset-chip[data-ref-kind='asset']",
  ) as HTMLElement | null;
  if (unityAssetRef) {
    const assetPath = normalizeAssetRefDatasetPath(
      unityAssetRef.dataset.assetPath || unityAssetRef.dataset.filePath,
    );
    if (assetPath && isUnityAssetPath(assetPath)) {
      return {
        kind: "asset",
        filePath: assetPath,
        assetPath,
      };
    }
    if (assetPath) {
      return {
        kind: "file",
        filePath: assetPath,
        entryKind: unityAssetRef.dataset.entryKind === "folder" ? "folder" : "file",
      };
    }
  }

  const workspaceAssetRef = target.closest(".md-workspace-ref[data-workspace-path]") as HTMLElement | null;
  const workspacePath = normalizeAssetRefDatasetPath(workspaceAssetRef?.dataset.workspacePath);
  const workspaceKnowledgeTarget = parseKnowledgeDocumentRefPath(workspacePath);
  if (workspaceKnowledgeTarget) return workspaceKnowledgeTarget;
  if (!workspacePath || !isUnityAssetPath(workspacePath)) {
    const fileRef = target.closest(".md-file-ref[data-file-path]") as HTMLElement | null;
    const filePath = normalizeAssetRefDatasetPath(fileRef?.dataset.filePath);
    const fileKnowledgeTarget = parseKnowledgeDocumentRefPath(filePath);
    if (fileKnowledgeTarget) return fileKnowledgeTarget;
    if (filePath) {
      return {
        kind: "file",
        filePath,
        entryKind: (fileRef?.dataset.entryKind || workspaceAssetRef?.dataset.entryKind) === "folder"
          ? "folder"
          : "file",
      };
    }
    if (workspacePath && workspaceAssetRef?.dataset.entryKind === "folder") {
      return {
        kind: "file",
        filePath: workspacePath,
        entryKind: "folder",
      };
    }
    return null;
  }

  return {
    kind: "asset",
    filePath: workspacePath,
    assetPath: workspacePath,
  };
}

function handleContentContextMenu(e: MouseEvent) {
  if (!(e.target instanceof Element)) return;
  if (e.target.closest(".unity-property-fence, [data-md-unity-property-fence='true']")) return;
  const selectionText = typeof window === "undefined"
    ? ""
    : selectionTextAtPoint(window.getSelection(), { x: e.clientX, y: e.clientY });
  if (!selectionText) {
    const target = assetContextTargetFromElement(e.target);
    if (target) {
      e.preventDefault();
      e.stopPropagation();
      closeMessageContextMenu();
      assetRefCtxMenu.value = {
        x: e.clientX,
        y: e.clientY,
        target,
      };
      return;
    }
  }

  const messageEl = e.target.closest("[data-chat-message-id]") as HTMLElement | null;
  const hitMessageId = messageEl?.dataset.chatMessageId?.trim();
  const groupEl = messageEl?.closest("[data-chat-message-group-end-id]") as HTMLElement | null;
  const groupEndMessageId = groupEl?.dataset.chatMessageGroupEndId?.trim();
  const resolvedMessageId =
    groupEl?.dataset.chatMessageGroupRole === "assistant" && groupEndMessageId
      ? groupEndMessageId
      : hitMessageId;
  const messageId =
    resolvedMessageId && props.messages.some((message) => message.id === resolvedMessageId)
      ? resolvedMessageId
      : null;
  if (!selectionText && !messageId) return;

  e.preventDefault();
  e.stopPropagation();
  closeAssetRefContextMenu();
  messageCtxMenu.value = {
    x: e.clientX,
    y: e.clientY,
    messageId,
    selectionText,
  };
}

function handleContentClick(e: MouseEvent) {
  closeMessageContextMenu();
  const target = e.target as HTMLElement;
  if (isInsidePassiveMarkdownUnityPreview(target)) return;
  if (target.tagName === "IMG") {
    e.preventDefault();
    openLightbox((target as HTMLImageElement).src);
    return;
  }
  const knowledgeRef = target.closest(".md-knowledge-ref[data-knowledge-path]") as HTMLElement | null;
  if (knowledgeRef) {
    e.preventDefault();
    const path = normalizeAssetRefDatasetPath(knowledgeRef.dataset.knowledgePath);
    const docType = toKnowledgeDocumentType(knowledgeRef.dataset.knowledgeType ?? "")
      ?? toKnowledgeDocumentType(path.split("/")[0] ?? "");
    if (docType && path) {
      handleKnowledgeRefClick(docType, path);
    }
    return;
  }
  const workspaceRef = target.closest(".md-workspace-ref") as HTMLElement | null;
  if (workspaceRef) {
    e.preventDefault();
    const workspacePath = workspaceRef.dataset.workspacePath;
    const entryKind = workspaceRef.dataset.entryKind;
    if (!workspacePath) return;
    const knowledgeTarget = parseKnowledgeDocumentRefPath(workspacePath);
    if (knowledgeTarget) {
      handleKnowledgeRefClick(knowledgeTarget.docType, knowledgeTarget.path);
      return;
    }
    if (entryKind === "folder") {
      handleFolderRefClick(workspacePath);
      return;
    }
    handleFileRefClick(workspacePath);
    return;
  }
  const chip = target.closest(".md-asset-chip") as HTMLElement | null;
  if (chip) {
    e.preventDefault();
    const assetPath = chip.dataset.assetPath;
    if (assetPath) {
      if (shouldOpenUnityAssetInspector(e, assetPath)) {
        handleUnityAssetInspectorClick(assetPath);
        return;
      }
      handleFileRefClick(assetPath);
    }
    return;
  }
  const sceneObjectRef = target.closest(".md-unity-scene-object-ref") as HTMLElement | null;
  if (sceneObjectRef) {
    e.preventDefault();
    const scenePath = sceneObjectRef.dataset.scenePath;
    const objectPath = sceneObjectRef.dataset.sceneObjectPath;
    if (!scenePath || !objectPath) return;
    if (shouldOpenUnitySceneObjectInspector(e, scenePath, objectPath)) {
      handleUnitySceneObjectInspectorClick(scenePath, objectPath);
      return;
    }
    handleUnitySceneObjectClick(scenePath, objectPath);
    return;
  }
  const fileRef = target.closest(".md-file-ref") as HTMLElement | null;
  if (fileRef) {
    e.preventDefault();
    const filePath = fileRef.dataset.filePath;
    if (!filePath) return;
    const knowledgeTarget = parseKnowledgeDocumentRefPath(filePath);
    if (knowledgeTarget) {
      handleKnowledgeRefClick(knowledgeTarget.docType, knowledgeTarget.path);
      return;
    }
    if (fileRef.dataset.entryKind === "folder") {
      handleFolderRefClick(filePath);
      return;
    }
    const assetPath = fileRef.dataset.assetPath || filePath;
    if (shouldOpenUnityAssetInspector(e, assetPath)) {
      handleUnityAssetInspectorClick(assetPath);
      return;
    }
    handleFileRefClick(filePath);
  }
}

function isInsidePassiveMarkdownUnityPreview(target: Element): boolean {
  return !!target.closest("[data-md-unity-passive='true']");
}

function handleKnowledgeRefClick(docType: KnowledgeDocumentType, path: string) {
  if (props.scopedSession) {
    const workspaceRef = resourceWorkspaceRef();
    if (workspaceRef) {
      emit("openKnowledgeDocument", { docType, path, workspaceRef });
    }
    return;
  }
  void openLegacyKnowledgeDocument(docType, path).catch((error) => {
    const err = normalizeAppError(error);
    notificationStore.addNotice("warning", t("chat.knowledgeRef.openFailed", err.message), {
      code: err.code,
      operation: "knowledgeRef",
      replaceOperation: true,
    });
  });
}

function handleUnityAssetInspectorClick(filePath: string) {
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return;
  openUnityAssetInspector(workspaceRef, filePath).catch((e: unknown) => {
    console.warn("openUnityAssetInspector failed:", e);
    handleFileRefClick(filePath);
  });
}

function handleUnitySceneObjectInspectorClick(scenePath: string, objectPath: string) {
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return;
  openUnitySceneObjectInspector(workspaceRef, scenePath, objectPath).catch((e: unknown) => {
    console.warn("openUnitySceneObjectInspector failed:", e);
    notifyUnitySceneObjectError(e, scenePath, objectPath);
  });
}

function handleUnitySceneObjectClick(scenePath: string, objectPath: string) {
  runAssetRefClickAction({ kind: "sceneObject", scenePath, objectPath });
}

function selectUnitySceneObjectRef(scenePath: string, objectPath: string) {
  if (!shouldUseUnitySceneObjectRef(scenePath, objectPath)) return;
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return;
  selectUnitySceneObject(workspaceRef, scenePath, objectPath).catch((e: unknown) => {
    console.warn("selectUnitySceneObject failed:", e);
    notifyUnitySceneObjectError(e, scenePath, objectPath);
  });
}

function notifyUnitySceneObjectError(error: unknown, scenePath: string, objectPath: string) {
  const kind = classifyUnitySceneObjectError(error);
  const message = kind === "sceneNotLoaded"
    ? t("chat.sceneObject.sceneNotLoaded", scenePath)
    : kind === "objectMissing"
      ? t("chat.sceneObject.objectMissing", objectPath)
      : t("chat.sceneObject.openFailed", `${scenePath}/${objectPath}`);
  notificationStore.addNotice("warning", message, {
    operation: "unitySceneObjectRef",
    code: `unity.sceneObject.${kind}`,
    replaceOperation: true,
  });
}

function handleFileRefClick(filePath: string) {
  if (isUnityAssetPath(filePath)) {
    runAssetRefClickAction({ kind: "asset", assetPath: filePath, entryKind: "file" });
    return;
  }
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return;
  openFileExternal(filePath, workspaceRef).catch((e: unknown) => console.warn("openFileExternal failed:", e));
}

function handleFolderRefClick(folderPath: string) {
  if (isUnityAssetPath(folderPath)) {
    runAssetRefClickAction({ kind: "asset", assetPath: folderPath, entryKind: "folder" });
    return;
  }
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return;
  showInFolder(folderPath, workspaceRef).catch((e: unknown) => console.warn("showInFolder failed:", e));
}

type AssetRefClickTarget =
  | { kind: "asset"; assetPath: string; entryKind: "file" | "folder" }
  | { kind: "sceneObject"; scenePath: string; objectPath: string };

function assetRefInspectorPayload(target: AssetRefClickTarget): LocusAssetInspectorTarget {
  return target.kind === "sceneObject"
    ? { kind: "sceneObject", scenePath: target.scenePath, objectPath: target.objectPath }
    : { assetPath: target.assetPath };
}

/** Pre-inspector click behavior, also the fallback when the inspector cannot open. */
function legacyAssetRefClick(target: AssetRefClickTarget) {
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return;
  if (target.kind === "sceneObject") {
    selectUnitySceneObjectRef(target.scenePath, target.objectPath);
    return;
  }
  if (target.entryKind === "folder") {
    if (shouldSelectUnityAsset(target.assetPath)) {
      selectUnityAsset(workspaceRef, target.assetPath).catch((e: unknown) => console.warn("selectUnityAsset failed:", e));
      return;
    }
    showInFolder(target.assetPath, workspaceRef).catch((e: unknown) => console.warn("showInFolder failed:", e));
    return;
  }
  if (canOpenInEditor(target.assetPath)) {
    openFileExternal(target.assetPath, workspaceRef).catch((e: unknown) => console.warn("openFileExternal failed:", e));
    return;
  }
  if (shouldSelectUnityAsset(target.assetPath)) {
    selectUnityAsset(workspaceRef, target.assetPath).catch((e: unknown) => console.warn("selectUnityAsset failed:", e));
    return;
  }
  openFileExternal(target.assetPath, workspaceRef).catch((e: unknown) => console.warn("openFileExternal failed:", e));
}

function openAssetRefInUnityInspector(target: AssetRefClickTarget) {
  if (target.kind === "sceneObject") {
    handleUnitySceneObjectInspectorClick(target.scenePath, target.objectPath);
    return;
  }
  const workspaceRef = resourceWorkspaceRef();
  if (!workspaceRef) return;
  openUnityAssetInspector(workspaceRef, target.assetPath).catch((e: unknown) => {
    console.warn("openUnityAssetInspector failed:", e);
    // Fall back to the pre-inspector behavior instead of handleFileRefClick,
    // which would re-enter runAssetRefClickAction and loop on repeat failure.
    legacyAssetRefClick(target);
  });
}

function runAssetRefClickAction(target: AssetRefClickTarget) {
  const action = isUnityEmbeddedWindow()
    ? displaySettings.unityEmbedAssetRefClickAction
    : displaySettings.assetRefClickAction;
  if (action === "unityInspector") {
    openAssetRefInUnityInspector(target);
    return;
  }
  if (action === "fileBrowser") {
    const revealPath = target.kind === "sceneObject" ? target.scenePath : target.assetPath;
    const workspaceRef = resourceWorkspaceRef();
    if (!workspaceRef) return;
    showInFolder(revealPath, workspaceRef).catch((e: unknown) => console.warn("showInFolder failed:", e));
    return;
  }
  if (action === "locusInspector") {
    openLocusAssetInspector(assetRefInspectorPayload(target))
      .then((opened) => {
        if (!opened) legacyAssetRefClick(target);
      })
      .catch((error: unknown) => {
        console.warn("openLocusAssetInspector failed:", error);
        notifyAssetRefContextMenuError(error, "assetRefOpenLocusInspector", t("asset.inspector.openFailed"));
      });
    return;
  }
  legacyAssetRefClick(target);
}

function notifyAssetRefContextMenuError(error: unknown, operation: string, fallbackMessage: string) {
  const err = normalizeAppError(error);
  notificationStore.addNotice("warning", err.message || fallbackMessage, {
    code: err.code,
    operation,
    replaceOperation: true,
  });
}

function assetRefContextCopyPath(target: AssetRefContextMenuTarget): string {
  if (target.kind === "sceneObject") {
    return `${target.scenePath}/${target.objectPath}`;
  }
  if (target.kind === "knowledge") {
    return target.path;
  }
  return target.filePath;
}

async function doAssetRefCopyPath() {
  const target = assetRefCtxMenu.value?.target;
  if (!target) return;
  const path = assetRefContextCopyPath(target);
  closeAssetRefContextMenu();
  try {
    await navigator.clipboard.writeText(path);
    notificationStore.addNotice("success", t("common.copied"), {
      operation: "assetRefCopyPath",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  } catch (error) {
    console.warn("copy asset ref path failed:", error);
    notifyAssetRefContextMenuError(error, "assetRefCopyPath", t("common.copyPathFailed"));
  }
}

async function doAssetRefOpenInEditor() {
  const target = assetRefCtxMenu.value?.target;
  if (!target || !assetRefContextCanOpenInEditor.value) return;
  closeAssetRefContextMenu();
  try {
    const workspaceRef = resourceWorkspaceRef();
    if (!workspaceRef) return;
    await openFileExternal(target.filePath, workspaceRef);
  } catch (error) {
    console.warn("openFileExternal failed:", error);
    notifyAssetRefContextMenuError(error, "assetRefOpenInEditor", "Failed to open file");
  }
}

async function doAssetRefOpenInLocusInspector() {
  const target = assetRefCtxMenu.value?.target;
  if (!target || !assetRefContextCanOpenLocusInspector.value) return;
  closeAssetRefContextMenu();
  try {
    if (target.kind === "sceneObject") {
      const opened = await openLocusAssetInspector({
        kind: "sceneObject",
        scenePath: target.scenePath,
        objectPath: target.objectPath,
      });
      if (!opened) {
        const workspaceRef = resourceWorkspaceRef();
        if (!workspaceRef) return;
        await openUnitySceneObjectInspector(workspaceRef, target.scenePath, target.objectPath);
      }
      return;
    }
    if (target.kind !== "asset") return;
    const opened = await openLocusAssetInspector({ assetPath: target.assetPath });
    if (!opened) {
      const workspaceRef = resourceWorkspaceRef();
      if (!workspaceRef) return;
      await openFileExternal(target.filePath, workspaceRef);
    }
  } catch (error) {
    console.warn("openLocusAssetInspector failed:", error);
    notifyAssetRefContextMenuError(error, "assetRefOpenLocusInspector", t("asset.inspector.openFailed"));
  }
}

async function openInlineDiffInWindow() {
  const payload = inlineDiffPayload.value;
  if (!payload) return;
  try {
    await openChatDiffReviewWindow({ payload });
  } catch (cause) {
    const err = normalizeAppError(cause);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openChatDiffReviewWindow",
    });
  }
}

function openInlineDiffFileExternal() {
  const payload = inlineDiffPayload.value;
  const workspaceRef = resourceWorkspaceRef();
  if (!payload || !workspaceRef) return;
  void openFileExternal(payload.filePath, workspaceRef).catch((error: unknown) => {
    console.warn("openFileExternal failed:", error);
  });
}

function handleContentPointerDown(event: PointerEvent) {
  if (event.button !== 0 || event.isPrimary === false) return;
  const workspaceRef = props.workspaceRef;
  if (!workspaceRef) return;
  const checkout = workspaceContextStore.checkoutsById[workspaceRef.checkoutId];
  const projectId = props.projectId?.trim() || checkout?.projectId || "";
  const workspaceRoot = props.workingDir?.trim() || checkout?.root || "";
  if (!projectId || !workspaceRoot) return;

  const started = startWorkbenchReferenceInternalDrag(internalDrag, event, {
    projectId,
    workspaceRef: { ...workspaceRef },
    workspaceRoot,
  });
  if (!started) return;
  closeAssetRefContextMenu();
  closeMessageContextMenu();
}

async function openCompactedContext(messageId: string) {
  const sessionId = props.activeSessionId?.trim();
  if (!sessionId || !messageId.trim()) return;
  try {
    await openContextCompactionWindow({ sessionId, messageId });
  } catch (cause) {
    const err = normalizeAppError(cause);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openContextCompactionWindow",
    });
  }
}

async function doAssetRefShowInFolder() {
  const target = assetRefCtxMenu.value?.target;
  if (!target) return;
  closeAssetRefContextMenu();
  try {
    if (target.kind === "knowledge") {
      const workspaceRef = resourceWorkspaceRef();
      if (!workspaceRef) return;
      await knowledgeRevealTarget({
        kind: "document",
        docType: target.docType,
        path: target.path,
      }, workspaceRef);
      return;
    }
    const workspaceRef = resourceWorkspaceRef();
    if (!workspaceRef) return;
    await showInFolder(target.filePath, workspaceRef);
  } catch (error) {
    console.warn("showInFolder failed:", error);
    notifyAssetRefContextMenuError(error, "assetRefShowInFolder", "Failed to show file in folder");
  }
}

function doAssetRefOpenInKnowledge() {
  const target = assetRefCtxMenu.value?.target;
  if (!target || target.kind !== "knowledge") return;
  closeAssetRefContextMenu();
  if (props.scopedSession) {
    const workspaceRef = resourceWorkspaceRef();
    if (workspaceRef) {
      emit("openKnowledgeDocumentInKnowledge", {
        docType: target.docType,
        path: target.path,
        workspaceRef,
      });
    }
    return;
  }
  openLegacyKnowledgeDocumentInKnowledge(target.docType, target.path);
}

async function doAssetRefSelectInUnity() {
  const target = assetRefCtxMenu.value?.target;
  if (!target || !assetRefContextCanSelectInUnity.value) return;
  closeAssetRefContextMenu();

  try {
    const workspaceRef = resourceWorkspaceRef();
    if (!workspaceRef) return;
    if (target.kind === "sceneObject") {
      await selectUnitySceneObject(workspaceRef, target.scenePath, target.objectPath);
      return;
    }
    if (target.kind !== "asset") return;
    await selectUnityAsset(workspaceRef, target.assetPath);
  } catch (error) {
    console.warn("selectUnityAsset failed:", error);
    if (target.kind === "sceneObject") {
      notifyUnitySceneObjectError(error, target.scenePath, target.objectPath);
      return;
    }
    notifyAssetRefContextMenuError(error, "assetRefSelectInUnity", "Failed to select asset in Unity");
  }
}

const messageContextMessage = computed(() => {
  const messageId = messageCtxMenu.value?.messageId;
  if (!messageId) return null;
  return props.messages.find((message) => message.id === messageId) ?? null;
});

const messageContextCanCopy = computed(() =>
  !!messageContextMessage.value && messageContextMessage.value.role !== "tool",
);

const messageContextCanAct = computed(() =>
  !!props.activeSessionId
  && !props.isStreaming
  && !!messageContextMessage.value
  && messageContextMessage.value.role !== "tool",
);

const effectiveCurrentRunId = computed(() => (
  props.scopedSession ? props.currentRunId ?? null : chatStore.currentRunId
));

const messageContextCanFork = computed(() =>
  !!props.activeSessionId
  && !!messageContextMessage.value
  && messageContextMessage.value.role !== "tool"
  && (!props.scopedSession || !!props.forkFromMessage)
  && !isMessageFromActiveStream(
    messageContextMessage.value,
    effectiveCurrentRunId.value,
    props.isStreaming,
  ),
);

function lastRenderableMessage() {
  for (let index = props.messages.length - 1; index >= 0; index -= 1) {
    const message = props.messages[index];
    if (message && hasRenderableTranscriptMessage(message)) {
      return message;
    }
  }
  return null;
}

function isLastUserMessageWithoutAssistantAfter(message: ChatMessage) {
  if (message.role !== "user") return false;
  if (lastRenderableMessage()?.id !== message.id) return false;
  const messageIndex = props.messages.findIndex((candidate) => candidate.id === message.id);
  if (messageIndex < 0) return false;
  return !props.messages
    .slice(messageIndex + 1)
    .some((candidate) => candidate.role === "assistant" && hasRenderableTranscriptMessage(candidate));
}

const messageContextShouldShowReEdit = computed(() => {
  const message = messageContextMessage.value;
  return !!message && isLastUserMessageWithoutAssistantAfter(message);
});

const messageContextCanReEdit = computed(() =>
  !!props.activeSessionId
  && !props.isStreaming
  && messageContextShouldShowReEdit.value
  && (
    !props.scopedSession
    || (!!props.undoConversation && !!props.restoreComposerDraft)
  ),
);

async function doMessageCopy() {
  const message = messageContextMessage.value;
  if (!message || !messageContextCanCopy.value) return;
  closeMessageContextMenu();
  try {
    await writeChatMessageClipboard(buildChatMessageClipboardPayload(message));
    notificationStore.addNotice("success", t("common.copied"), {
      operation: "messageCopy",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  } catch (error) {
    console.warn("copy chat message failed:", error);
    notificationStore.addNotice("warning", t("chat.messageMenu.copyFailed"), {
      operation: "messageCopy",
      replaceOperation: true,
    });
  }
}

async function doMessageCopySelection() {
  const selectionText = messageCtxMenu.value?.selectionText ?? "";
  if (!selectionText) return;
  closeMessageContextMenu();
  try {
    await writeChatMessageClipboard({ text: selectionText, draft: null, serializedDraft: null });
    notificationStore.addNotice("success", t("common.copied"), {
      operation: "messageCopy",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  } catch (error) {
    console.warn("copy chat selection failed:", error);
    notificationStore.addNotice("warning", t("chat.messageMenu.copyFailed"), {
      operation: "messageCopy",
      replaceOperation: true,
    });
  }
}

async function doMessageReEdit() {
  const message = messageContextMessage.value;
  if (!message || !messageContextCanReEdit.value) return;
  const draft = buildUserMessageDraft(message);
  closeMessageContextMenu();
  chatChangesStore.closeInlineDiffForSession(props.activeSessionId);
  const undo = props.scopedSession
    ? props.undoConversation
    : () => chatStore.undoLatestConversationTurn();
  if (!undo) return;
  const undone = await undo(null);
  if (undone) {
    await restoreUserMessageDraft(draft);
  }
}

async function doMessageRollback() {
  const messageId = messageCtxMenu.value?.messageId ?? null;
  if (!messageId || !messageContextCanAct.value) return;
  closeMessageContextMenu();
  await openUndoChooser(messageId);
}

async function doMessageFork() {
  const messageId = messageCtxMenu.value?.messageId ?? null;
  if (!messageId || !messageContextCanFork.value) return;
  closeMessageContextMenu();
  if (props.scopedSession) {
    await props.forkFromMessage?.(messageId);
    return;
  }
  await chatStore.forkSessionFromMessage(messageId);
}

function handleQuestionAnswer(answer: string) {
  emit("answerQuestion", answer);
}

const NEW_CHAT_DRAFT_KEY = "__new_chat__";
const inputText = ref(props.composerValue ?? "");
const composerDrafts = ref(new Map<string, string>());
const composerPanelRef = ref<InstanceType<typeof RichChatInput> | null>(null);
const transcriptRef = ref<InstanceType<typeof ChatTranscript> | null>(null);
const transcriptScrollElement = computed(() => transcriptRef.value?.getScrollElement() ?? null);

function draftSessionKey(sessionId: string | null) {
  return sessionId ?? NEW_CHAT_DRAFT_KEY;
}

const sessionSurfaceKey = computed(() => (
  props.sessionSurfaceKey?.trim() || draftSessionKey(props.activeSessionId)
));
const composerAssetRefSyncKey = computed(() => `chat:${sessionSurfaceKey.value}`);
type UndoChoice = "conversation" | "files";
const undoChooserVisible = ref(false);
const undoChooserRef = ref<HTMLElement | null>(null);
const selectedUndoChoice = ref<UndoChoice>("conversation");
const undoAction = ref<"conversation" | "files" | null>(null);
const undoChooserBusy = computed(() => undoAction.value !== null);
const undoTargetMessageId = ref<string | null>(null);
const contextSelectedMessageId = computed(() => messageCtxMenu.value?.messageId ?? undoTargetMessageId.value);

const latestConversationTurn = computed(() => {
  let userIndex = -1;
  for (let index = props.messages.length - 1; index >= 0; index -= 1) {
    if (props.messages[index]?.role === "user") {
      userIndex = index;
      break;
    }
  }
  if (userIndex < 0) return null;

  const turnMessages = props.messages.slice(userIndex);
  const fileUndoTarget = turnMessages.find(
    (message) => message.role === "assistant" && !!props.undoableMessageIds?.has(message.id),
  )?.id ?? null;

  return {
    userMessage: props.messages[userIndex] ?? null,
    fileUndoTarget,
  };
});

function rollbackTargetForMessage(messageId: string) {
  const messageIndex = props.messages.findIndex((message) => message.id === messageId);
  if (messageIndex < 0) return null;
  const message = props.messages[messageIndex];
  if (!message || message.role === "tool") return null;

  const rollbackMessages = props.messages.slice(messageIndex + 1);
  const fileUndoTarget = rollbackMessages.find(
    (candidate) => candidate.role === "assistant" && !!props.undoableMessageIds?.has(candidate.id),
  )?.id ?? null;

  return {
    messageId,
    userMessage: null,
    fileUndoTarget,
  };
}

const currentUndoTarget = computed(() => {
  if (undoTargetMessageId.value) {
    return rollbackTargetForMessage(undoTargetMessageId.value);
  }
  return latestConversationTurn.value;
});

const canUndoConversation = computed(() =>
  !!props.activeSessionId
  && !!currentUndoTarget.value
  && !props.isStreaming
  && (
    !props.scopedSession
    || (
      !!props.undoConversation
      && (!currentUndoTarget.value.userMessage || !!props.restoreComposerDraft)
    )
  ),
);
const canUndoFilesAndConversation = computed(() =>
  canUndoConversation.value
  && !!currentUndoTarget.value?.fileUndoTarget
  && (!props.scopedSession || !!props.undoFilesAndConversation),
);
const undoChoices = computed<UndoChoice[]>(() => {
  if (!canUndoConversation.value) return [];
  return canUndoFilesAndConversation.value
    ? ["conversation", "files"]
    : ["conversation"];
});

watch(undoChoices, (choices) => {
  if (!undoChooserVisible.value || choices.length === 0) return;
  if (!choices.includes(selectedUndoChoice.value)) {
    selectedUndoChoice.value = defaultUndoChoice();
  }
});

function storeComposerDraft(sessionId: string | null, value: string) {
  const key = draftSessionKey(sessionId);
  if (value) {
    composerDrafts.value.set(key, value);
    return;
  }
  composerDrafts.value.delete(key);
}

async function restoreComposerDraft(sessionId: string | null) {
  if (props.scopedSession && props.composerValue !== undefined) {
    inputText.value = props.composerValue;
    await nextTick();
    composerPanelRef.value?.resizeTextarea();
    return;
  }
  inputText.value = composerDrafts.value.get(draftSessionKey(sessionId)) ?? "";
  await nextTick();
  composerPanelRef.value?.resizeTextarea();
}

async function focusComposerInput() {
  await nextTick();
  composerPanelRef.value?.resizeTextarea();
  const end = inputText.value.length;
  composerPanelRef.value?.focus();
  composerPanelRef.value?.setSelectionRange(end, end);
}

function defaultUndoChoice(): UndoChoice {
  return canUndoFilesAndConversation.value ? "files" : "conversation";
}

const undoChooserDirtyFiles = ref<ChangedFile[]>([]);
/** Whether the dirty preflight for the current chooser target completed. */
const undoChooserDirtyChecked = ref(false);
const undoChooserDirtyPreview = computed(() => {
  const paths = undoChooserDirtyFiles.value.map((file) => file.path);
  if (paths.length <= 5) return paths.join("\n");
  return [...paths.slice(0, 5), `+${paths.length - 5}`].join("\n");
});

function refreshUndoChooserDirtyState() {
  undoChooserDirtyFiles.value = [];
  undoChooserDirtyChecked.value = false;
  const targetId = currentUndoTarget.value?.fileUndoTarget;
  if (!targetId) return;
  const checkDirty = props.scopedSession
    ? props.checkUndoDirty
    : props.checkUndoDirty ?? chatStore.checkUndoDirty;
  if (!checkDirty) return;
  void checkDirty(targetId).then(
    (files) => {
      if (currentUndoTarget.value?.fileUndoTarget !== targetId) return;
      undoChooserDirtyFiles.value = files;
      undoChooserDirtyChecked.value = true;
    },
    (e) => {
      console.warn("[ChatView] undo_check_dirty failed:", e);
    },
  );
}

async function openUndoChooser(messageId: string | null = null) {
  undoTargetMessageId.value = messageId;
  selectedUndoChoice.value = defaultUndoChoice();
  undoChooserVisible.value = true;
  refreshUndoChooserDirtyState();
  await nextTick();
  undoChooserRef.value?.focus();
}

function closeUndoChooser() {
  if (undoChooserBusy.value) return;
  undoChooserVisible.value = false;
  undoTargetMessageId.value = null;
  undoChooserDirtyFiles.value = [];
  undoChooserDirtyChecked.value = false;
}

async function restoreUserMessageDraft(draft: UserMessageDraft) {
  if (props.scopedSession) {
    await props.restoreComposerDraft?.(draft);
    return;
  }
  uiStore.stageChatDraftPrefill(draft);
}

async function restoreUndoMessage(message: ChatMessage | null) {
  if (!message) return;
  await restoreUserMessageDraft(buildUserMessageDraft(message));
}

function moveUndoChoice(delta: number) {
  const choices = undoChoices.value;
  if (choices.length === 0) return;
  const currentIndex = choices.indexOf(selectedUndoChoice.value);
  const nextIndex = currentIndex < 0
    ? 0
    : (currentIndex + delta + choices.length) % choices.length;
  selectedUndoChoice.value = choices[nextIndex] ?? defaultUndoChoice();
}

function runSelectedUndoChoice() {
  if (selectedUndoChoice.value === "files" && canUndoFilesAndConversation.value) {
    void undoFilesAndConversation();
    return;
  }
  if (canUndoConversation.value) {
    void undoConversationOnly();
  }
}

function handleUndoChooserKeydown(event: KeyboardEvent) {
  if (!undoChooserVisible.value) return;
  if (event.key === "Escape") {
    event.preventDefault();
    closeUndoChooser();
    return;
  }
  if (undoChooserBusy.value) return;
  if (event.key === "ArrowDown") {
    event.preventDefault();
    moveUndoChoice(1);
    return;
  }
  if (event.key === "ArrowUp") {
    event.preventDefault();
    moveUndoChoice(-1);
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    runSelectedUndoChoice();
  }
}

async function undoConversationOnly() {
  const targetMessageId = undoTargetMessageId.value;
  const turn = currentUndoTarget.value;
  if (!turn || !canUndoConversation.value || undoChooserBusy.value) return;
  undoAction.value = "conversation";
  chatChangesStore.closeInlineDiffForSession(props.activeSessionId);
  try {
    const undo = props.scopedSession
      ? props.undoConversation
      : props.undoConversation
        ?? (
          targetMessageId
            ? (messageId: string | null) => chatStore.rollbackToMessage(messageId!, { includeFiles: false })
            : () => chatStore.undoLatestConversationTurn()
        );
    if (!undo) return;
    const undone = await undo(targetMessageId);
    if (undone) {
      undoChooserVisible.value = false;
      undoTargetMessageId.value = null;
      await restoreUndoMessage(turn.userMessage);
    }
  } finally {
    undoAction.value = null;
  }
}

async function undoFilesAndConversation() {
  const targetMessageId = undoTargetMessageId.value;
  const turn = currentUndoTarget.value;
  const targetId = turn?.fileUndoTarget;
  if (!targetId || !canUndoFilesAndConversation.value || undoChooserBusy.value) return;
  undoAction.value = "files";
  chatChangesStore.closeInlineDiffForSession(props.activeSessionId);
  // The chooser showed the dirty file list (or verified it empty); if the
  // preflight never completed, leave acceptDirty off so the backend re-checks.
  const acceptDirty = undoChooserDirtyChecked.value;
  try {
    const undo = props.scopedSession
      ? props.undoFilesAndConversation
      : props.undoFilesAndConversation
        ?? (
          targetMessageId
            ? (messageId: string | null, fileUndoTarget: string, accept: boolean) => (
                chatStore.rollbackToMessage(messageId!, {
                  includeFiles: true,
                  fileUndoTarget,
                  acceptDirty: accept,
                })
              )
            : (_messageId: string | null, fileUndoTarget: string, accept: boolean) => (
                chatStore.performUndo(fileUndoTarget, { acceptDirty: accept })
              )
        );
    if (!undo) return;
    const undone = await undo(targetMessageId, targetId, acceptDirty);
    if (undone) {
      undoChooserVisible.value = false;
      undoTargetMessageId.value = null;
      undoChooserDirtyFiles.value = [];
      undoChooserDirtyChecked.value = false;
      await restoreUndoMessage(turn.userMessage);
    }
  } finally {
    undoAction.value = null;
  }
}

async function handleNewChatRequest(source: NewChatRequestSource = "control") {
  const shortcutAction = source === "shortcut"
    ? props.newChatShortcutAction ?? "replaceCurrent"
    : "replaceCurrent";
  if (shortcutAction === "keepCurrent") {
    await focusComposerInput();
    return;
  }
  if (shortcutAction !== "newTab") {
    if (props.activeSessionId === null) {
      composerPanelRef.value?.resetDraft();
      inputText.value = "";
    }
  }
  emit("newChat", { source });
  if (shortcutAction === "newTab") return;
  await nextTick();
  await focusComposerInput();
}

async function applyExternalComposerPrefill(text: string) {
  if (composerPanelRef.value) {
    await composerPanelRef.value.applyPrefill(text);
    return;
  }
  inputText.value = text;
  await focusComposerInput();
}

function isComposerDraftEmpty() {
  return composerPanelRef.value?.isDraftEmpty() ?? inputText.value.length === 0;
}

async function applyDraftPrefill(draft: UserMessageDraft) {
  if (composerPanelRef.value) {
    await composerPanelRef.value.applyDraftPrefill(draft);
    return;
  }
  inputText.value = draft.text;
  await focusComposerInput();
}

async function appendComposerDraft(draft: UserMessageDraft) {
  if (composerPanelRef.value) {
    await composerPanelRef.value.appendDraft(draft);
    return;
  }
  inputText.value += draft.text;
  await focusComposerInput();
}

function exportComposerDraft(): UserMessageDraft {
  return composerPanelRef.value?.exportDraft() ?? {
    text: inputText.value,
    images: [],
    assetRefs: [],
    localFiles: [],
    consoleTexts: [],
    intent: emptyComposerIntent(),
  };
}

defineExpose({
  applyDraftPrefill,
  appendComposerDraft,
  applyExternalComposerPrefill,
  exportComposerDraft,
  focusComposerInput,
  isComposerDraftEmpty,
});

function canApplyPendingChatPrefill(prefill: NonNullable<typeof uiStore.pendingChatPrefill>) {
  if (prefill.sessionId !== undefined && prefill.sessionId !== props.activeSessionId) {
    return false;
  }
  if (prefill.requireEmptyComposer && !isComposerDraftEmpty()) {
    return false;
  }
  return true;
}

watch(
  () => uiStore.pendingChatPrefill?.id,
  async (prefillId) => {
    if (props.scopedSession) return;
    const prefill = uiStore.pendingChatPrefill;
    if (!prefillId || !prefill) return;
    if (!canApplyPendingChatPrefill(prefill)) {
      uiStore.clearPendingChatPrefill(prefillId);
      return;
    }
    if (prefill.draft) {
      await nextTick();
      if (!canApplyPendingChatPrefill(prefill)) {
        uiStore.clearPendingChatPrefill(prefillId);
        return;
      }
      if (composerPanelRef.value) {
        await composerPanelRef.value.applyDraftPrefill(prefill.draft);
      } else {
        await applyExternalComposerPrefill(prefill.text);
      }
    } else {
      await applyExternalComposerPrefill(prefill.text);
    }
    uiStore.clearPendingChatPrefill(prefillId);
  },
);

watch(inputText, (value) => {
  storeComposerDraft(props.activeSessionId, value);
  if (props.scopedSession) emit("updateComposerValue", value);
});

watch(
  () => props.composerValue,
  (value) => {
    if (!props.scopedSession || value === undefined || value === inputText.value) return;
    inputText.value = value;
    nextTick(() => composerPanelRef.value?.resizeTextarea());
  },
);

const hasStreamingContent = computed(
  () => (props.hasStreamingText ?? !!displayedStreamingText.value)
    || hasRunningToolCall(props.activeToolCalls)
);

const isWaitingForResponse = computed(
  () => shouldShowWaitingPlaceholder({
    isStreaming: props.isStreaming,
    hasStreamingContent: hasStreamingContent.value,
    isThinking: props.isThinking,
    hasThinkingContent: props.hasThinking,
  })
);

function hasRenderableTranscriptMessage(message: ChatMessage) {
  if (message.role === "tool") return false;
  const knowledgeStatus = message.knowledgeProposal?.status;
  if (knowledgeStatus === "stale" || knowledgeStatus === "invalidated") {
    return false;
  }

  if (message.role === "user") {
    return !!(
      message.content
      || (message.images && message.images.length > 0)
      || (message.assetRefs && message.assetRefs.length > 0)
      || message.intentMeta?.mode
      || (message.intentMeta?.skills && message.intentMeta.skills.length > 0)
    );
  }

  return !!(
    message.content
    || message.thinkingContent
    || (message.toolCalls && message.toolCalls.length > 0)
    || message.knowledgeProposal
  );
}

const showWelcomeState = computed(
  () =>
    !props.messages.some((message) => hasRenderableTranscriptMessage(message))
    && !hasStreamingContent.value
    && !props.isThinking
    && !props.hasThinking
    && !isWaitingForResponse.value,
);

const pendingRestoreSessionId = ref<string | null>(null);
const pendingRestoreMessagesRef = ref<ChatMessage[] | null>(null);
const toolHandoffViewportQuiet = ref(false);
let sessionRestoreFrame = 0;
let suppressScrollCapture = false;
let activeToolViewportAnchor: LiveScrollAnchorSnapshot | null = null;
let toolViewportAnchorFrame = 0;
const displayedStreamingText = ref("");
let pendingStreamingText = "";
let streamingTextFlushTimer: ReturnType<typeof setTimeout> | null = null;
let sessionRestoreLayoutTimer: ReturnType<typeof setTimeout> | null = null;
const userScrollIntent = createUserScrollIntentTracker();
const turnNavigationRevealActive = ref(false);
const STREAMING_TEXT_RENDER_DELAY_MS = STREAMING_RENDER_THROTTLE_MS;
const STREAM_END_SCROLL_SETTLE_MS = 320;
const SESSION_RESTORE_LAYOUT_STABILIZE_MS = 0;
const SESSION_RESTORE_MAX_SETTLE_FRAMES = 12;
const SESSION_RESTORE_REQUIRED_STABLE_FRAMES = 2;
const sessionRestoreLayoutStabilizing = ref(false);
const sessionRestoreViewportGuarding = ref(false);
const scopedSessionScrollStates = new Map<string, SessionScrollState>();

function scopedScrollStateKey(sessionId: string) {
  return props.sessionSurfaceKey?.trim() || sessionId;
}

function getSessionScrollState(sessionId: string): SessionScrollState | null {
  if (!props.scopedSession) return chatStore.getSessionScrollState(sessionId);
  return scopedSessionScrollStates.get(scopedScrollStateKey(sessionId)) ?? null;
}

function rememberSessionScrollState(sessionId: string, state: SessionScrollState) {
  if (!props.scopedSession) {
    chatStore.rememberSessionScrollState(sessionId, state);
    return;
  }
  scopedSessionScrollStates.set(scopedScrollStateKey(sessionId), state);
}

function clearStreamingTextFlushTimer() {
  if (!streamingTextFlushTimer) return;
  clearTimeout(streamingTextFlushTimer);
  streamingTextFlushTimer = null;
}

function clearSessionRestoreLayoutTimer() {
  if (!sessionRestoreLayoutTimer) return;
  clearTimeout(sessionRestoreLayoutTimer);
  sessionRestoreLayoutTimer = null;
}

function beginSessionRestoreLayoutStabilization() {
  clearSessionRestoreLayoutTimer();
  sessionRestoreLayoutStabilizing.value = true;
  sessionRestoreViewportGuarding.value = true;
}

function finishSessionRestoreLayoutStabilization(
  finalRestore?: {
    targetSessionId: string;
    state: SessionScrollState | null;
  },
) {
  clearSessionRestoreLayoutTimer();
  sessionRestoreLayoutTimer = setTimeout(() => {
    sessionRestoreLayoutTimer = null;
    sessionRestoreLayoutStabilizing.value = false;
    if (!finalRestore) {
      sessionRestoreViewportGuarding.value = false;
      return;
    }

    nextTick(() => {
      let frameCount = 0;
      let stableFrameCount = 0;
      let previousMetricsKey = "";
      const restoreUntilSettled = () => {
        sessionRestoreFrame = 0;
        if (props.activeSessionId !== finalRestore.targetSessionId) {
          sessionRestoreViewportGuarding.value = false;
          return;
        }

        restoreMessagesScrollState(finalRestore.state, finalRestore.targetSessionId);
        const metrics = readSessionScrollMetrics();
        const metricsKey = metrics
          ? `${metrics.scrollTop}:${metrics.scrollHeight}:${metrics.clientHeight}`
          : "";
        const expectsBottom = !finalRestore.state || finalRestore.state.mode === "bottom";
        const reachedTarget = !expectsBottom || (metrics?.distanceFromBottom ?? 0) <= 1;
        stableFrameCount = reachedTarget && metricsKey === previousMetricsKey
          ? stableFrameCount + 1
          : 0;
        previousMetricsKey = metricsKey;
        frameCount += 1;

        if (
          frameCount < SESSION_RESTORE_MAX_SETTLE_FRAMES
          && stableFrameCount < SESSION_RESTORE_REQUIRED_STABLE_FRAMES
        ) {
          sessionRestoreFrame = requestViewportFrame(restoreUntilSettled);
          return;
        }
        sessionRestoreViewportGuarding.value = false;
      };
      sessionRestoreFrame = requestViewportFrame(restoreUntilSettled);
    });
  }, SESSION_RESTORE_LAYOUT_STABILIZE_MS);
}

function cancelSessionRestoreLayoutStabilization() {
  clearSessionRestoreLayoutTimer();
  if (!sessionRestoreLayoutStabilizing.value && !sessionRestoreViewportGuarding.value) return;
  sessionRestoreLayoutStabilizing.value = false;
  sessionRestoreViewportGuarding.value = false;
}

function isSessionRestoreViewportGuardActive() {
  return !!pendingRestoreSessionId.value || sessionRestoreViewportGuarding.value;
}

function flushDisplayedStreamingText() {
  logToolCollapseTrace("chat-view", "flushDisplayedStreamingText", () => ({
    pendingLen: pendingStreamingText.length,
    pendingPreview: previewTraceText(pendingStreamingText, 64),
    previousDisplayedLen: displayedStreamingText.value.length,
  }));
  displayedStreamingText.value = pendingStreamingText;
  streamingTextFlushTimer = null;
}

watch(
  () => props.streamingText,
  (nextText, previousText = "") => {
    pendingStreamingText = nextText;
    logToolCollapseTrace("chat-view", "sourceStreamingTextChanged", () => ({
      previousLen: previousText.length,
      nextLen: nextText.length,
      displayedLen: displayedStreamingText.value.length,
      hasFlushTimer: !!streamingTextFlushTimer,
      nextPreview: nextText ? previewTraceText(nextText, 64) : "",
    }));
    if (!nextText || nextText.length < displayedStreamingText.value.length) {
      clearStreamingTextFlushTimer();
      logToolCollapseTrace("chat-view", "syncDisplayedStreamingTextImmediately", {
        reason: !nextText ? "empty" : "shrinking",
        nextLen: nextText.length,
        previousDisplayedLen: displayedStreamingText.value.length,
      });
      displayedStreamingText.value = nextText;
      return;
    }
    if (streamingTextFlushTimer) {
      logToolCollapseTrace("chat-view", "skipStreamingTextReschedule", {
        pendingLen: pendingStreamingText.length,
        displayedLen: displayedStreamingText.value.length,
      });
      return;
    }
    logToolCollapseTrace("chat-view", "scheduleDisplayedStreamingTextFlush", {
      delayMs: STREAMING_TEXT_RENDER_DELAY_MS,
      nextLen: nextText.length,
      displayedLen: displayedStreamingText.value.length,
    });
    streamingTextFlushTimer = setTimeout(flushDisplayedStreamingText, STREAMING_TEXT_RENDER_DELAY_MS);
  },
  { immediate: true },
);

function readMessageMetrics(el: HTMLElement) {
  return {
    scrollTop: el.scrollTop,
    clientHeight: el.clientHeight,
    scrollHeight: el.scrollHeight,
  };
}

function getMessagesElement() {
  return transcriptRef.value?.getScrollElement() ?? null;
}

function getMessagesContentElement() {
  return transcriptRef.value?.getContentElement?.() ?? null;
}

function roundScrollValue(value: number): number {
  return Math.round(value * 10) / 10;
}

function readSessionScrollMetrics(el: HTMLElement | null = getMessagesElement()) {
  if (!el) return null;
  const metrics = readMessageMetrics(el);
  const maxScrollTop = Math.max(0, metrics.scrollHeight - metrics.clientHeight);
  return {
    scrollTop: roundScrollValue(metrics.scrollTop),
    clientHeight: roundScrollValue(metrics.clientHeight),
    scrollHeight: roundScrollValue(metrics.scrollHeight),
    maxScrollTop: roundScrollValue(maxScrollTop),
    distanceFromBottom: roundScrollValue(metrics.scrollHeight - (metrics.scrollTop + metrics.clientHeight)),
  };
}

function traceToolViewportAnchor(phase: string, anchor: HTMLElement | null | undefined, detail: Record<string, unknown> = {}) {
  traceViewportAnchorSample({
    scope: "chat-view",
    phase,
    scrollElement: getMessagesElement(),
    contentElement: getMessagesContentElement(),
    anchor,
    detail: {
      sessionId: props.activeSessionId ?? "",
      ...detail,
    },
  });
}

function handleBottomPanelWheel(event: WheelEvent) {
  markMessagesUserScrollIntent();
  forwardWheelToElement(event, getMessagesElement());
}

function markMessagesUserScrollIntent() {
  userScrollIntent.mark();
  preserveMessagesViewportForUserScroll();
  requestOlderHistoryAtTop();
}

function captureCurrentSessionScrollState(el: HTMLElement): ReturnType<typeof captureSessionScrollState> {
  return captureSessionScrollState(readMessageMetrics(el), captureScrollAnchor(el));
}

function rememberScrollForSession(sessionId: string | null = props.activeSessionId) {
  const el = getMessagesElement();
  if (!sessionId || !el) return;
  rememberSessionScrollState(sessionId, captureCurrentSessionScrollState(el));
}

function runProgrammaticScrollUpdate(
  update: (el: HTMLElement) => void,
  sessionId: string | null = props.activeSessionId,
) {
  const el = getMessagesElement();
  if (!el) return;

  suppressScrollCapture = true;
  update(el);

  if (sessionId) {
    rememberSessionScrollState(sessionId, captureCurrentSessionScrollState(el));
  }

  requestAnimationFrame(() => {
    suppressScrollCapture = false;
  });
}

function requestViewportFrame(callback: () => void): number {
  if (typeof requestAnimationFrame === "function") {
    return requestAnimationFrame(() => callback());
  }
  return window.setTimeout(callback, 16);
}

function cancelViewportFrame(handle: number) {
  if (typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(handle);
    return;
  }
  window.clearTimeout(handle);
}

function cancelSessionRestoreFrame() {
  if (!sessionRestoreFrame) return;
  cancelViewportFrame(sessionRestoreFrame);
  sessionRestoreFrame = 0;
}

function clearToolViewportAnchorFrame() {
  if (!toolViewportAnchorFrame) return;
  cancelViewportFrame(toolViewportAnchorFrame);
  toolViewportAnchorFrame = 0;
}

function clearToolViewportAnchor() {
  clearToolViewportAnchorFrame();
  activeToolViewportAnchor = null;
}

function restoreToolViewportAnchor() {
  const anchorState = activeToolViewportAnchor;
  const el = getMessagesElement();
  if (!anchorState || !el) return false;
  if (isSessionRestoreViewportGuardActive()) {
    clearToolViewportAnchor();
    return false;
  }
  if (!el.contains(anchorState.anchor)) {
    traceToolViewportAnchor("restore:anchor-disconnected", anchorState.anchor);
    clearToolViewportAnchor();
    return false;
  }

  const before = captureTranscriptLayoutSnapshot("chat-view", "toolViewportAnchorRestore", el, getMessagesContentElement());
  suppressScrollCapture = true;
  const scrollTopBefore = el.scrollTop;
  const restored = restoreLiveScrollAnchor(el, anchorState);
  traceViewportAnchorSample({
    scope: "chat-view",
    phase: "restore",
    scrollElement: el,
    contentElement: getMessagesContentElement(),
    anchor: anchorState.anchor,
    anchorState: {
      offsetTop: anchorState.offsetTop,
      fallbackScrollTop: anchorState.fallbackScrollTop,
    },
    restored,
    before,
    detail: {
      sessionId: props.activeSessionId ?? "",
      scrollTopBefore,
      scrollTopAfter: el.scrollTop,
      scrollDelta: el.scrollTop - scrollTopBefore,
    },
  });
  if (restored && props.activeSessionId) {
    rememberSessionScrollState(props.activeSessionId, captureCurrentSessionScrollState(el));
  }

  requestViewportFrame(() => {
    suppressScrollCapture = false;
  });

  if (!restored) {
    clearToolViewportAnchor();
  }
  return restored;
}

function handleToolViewportAnchorStart(anchor: HTMLElement) {
  const el = getMessagesElement();
  if (!el || !el.contains(anchor)) return;
  if (isSessionRestoreViewportGuardActive()) {
    traceToolViewportAnchor("start:skip-session-restore", anchor, {
      pendingRestoreSessionId: pendingRestoreSessionId.value,
      sessionRestoreLayoutStabilizing: sessionRestoreLayoutStabilizing.value,
    });
    clearToolViewportAnchor();
    return;
  }

  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  if (!pendingStreamEndViewport) streamEndScrollScheduler.cancel();
  clearToolViewportAnchorFrame();
  activeToolViewportAnchor = captureLiveScrollAnchor(el, anchor);
  traceViewportAnchorSample({
    scope: "chat-view",
    phase: "start",
    scrollElement: el,
    contentElement: getMessagesContentElement(),
    anchor,
    anchorState: activeToolViewportAnchor
      ? {
          offsetTop: activeToolViewportAnchor.offsetTop,
          fallbackScrollTop: activeToolViewportAnchor.fallbackScrollTop,
        }
      : null,
    detail: {
      sessionId: props.activeSessionId ?? "",
    },
  });
  restoreToolViewportAnchor();
}

function handleToolViewportAnchorEnd(anchor: HTMLElement) {
  if (!activeToolViewportAnchor || activeToolViewportAnchor.anchor !== anchor) return;
  if (isSessionRestoreViewportGuardActive()) {
    traceToolViewportAnchor("end:skip-session-restore", anchor, {
      pendingRestoreSessionId: pendingRestoreSessionId.value,
      sessionRestoreLayoutStabilizing: sessionRestoreLayoutStabilizing.value,
    });
    clearToolViewportAnchor();
    return;
  }

  traceToolViewportAnchor("end:before-restore", anchor);
  restoreToolViewportAnchor();
  clearToolViewportAnchorFrame();
  toolViewportAnchorFrame = requestViewportFrame(() => {
    toolViewportAnchorFrame = 0;
    traceToolViewportAnchor("end:frame-before-restore", anchor);
    restoreToolViewportAnchor();
    traceToolViewportAnchor("end:frame-after-restore", anchor);
    activeToolViewportAnchor = null;
  });
}

function setMessagesScrollTop(scrollTop: number, sessionId: string | null = props.activeSessionId) {
  runProgrammaticScrollUpdate((el) => {
    el.scrollTop = scrollTop;
  }, sessionId);
}

function restoreMessagesScrollState(
  state: SessionScrollState | null,
  sessionId: string | null = props.activeSessionId,
) {
  const el = getMessagesElement();
  if (!el) return;

  const nextScrollTop = resolveSessionScrollTop(readMessageMetrics(el), state);
  runProgrammaticScrollUpdate((element) => {
    if (!restoreScrollAnchor(element, state)) {
      element.scrollTop = nextScrollTop;
    }
  }, sessionId);
}

function resolvePendingSessionRestoreState(state: SessionScrollState | null): SessionScrollState | null {
  const el = getMessagesElement();
  if (
    el
    && shouldRestoreBottomFromTopAnchorState(
      state,
      props.messages[0]?.id ?? null,
      readMessageMetrics(el),
    )
  ) {
    return { mode: "bottom" };
  }
  return state;
}

function isPendingSessionRestoreAwaitingMessages() {
  const targetSessionId = pendingRestoreSessionId.value;
  return !!targetSessionId
    && targetSessionId === props.activeSessionId
    && pendingRestoreMessagesRef.value === props.messages;
}

function scrollToBottomNow(force = false) {
  if (isPendingSessionRestoreAwaitingMessages()) {
    return;
  }

  const el = getMessagesElement();
  if (!el) return;

  const metrics = readMessageMetrics(el);
  const remembered = props.activeSessionId ? getSessionScrollState(props.activeSessionId) : null;
  if (!shouldAutoScrollToBottom({ force, metrics, remembered })) {
    return;
  }

  setMessagesScrollTop(resolveSessionScrollTop(metrics, { mode: "bottom" }));
}

const scrollToBottomScheduler = createCoalescedScrollScheduler((force) => {
  nextTick(() => {
    scrollToBottomNow(force);
  });
});

const preserveScrollAnchorScheduler = createCoalescedScrollScheduler(() => {
  nextTick(() => {
    if (isPendingSessionRestoreAwaitingMessages()) return;
    if (isSessionRestoreViewportGuardActive()) {
      return;
    }

    const sessionId = props.activeSessionId;
    const remembered = sessionId ? getSessionScrollState(sessionId) : null;
    if (!remembered || remembered.mode === "bottom") return;
    restoreMessagesScrollState(remembered, sessionId);
  });
});

function scrollToBottom(force = false) {
  scrollToBottomScheduler.schedule(force);
}

function preserveScrollAnchor() {
  preserveScrollAnchorScheduler.schedule();
}

interface StreamEndViewportSnapshot {
  sessionId: string | null;
  followBottom: boolean;
  state: SessionScrollState;
  settleAfter: number;
}

let pendingStreamEndViewport: StreamEndViewportSnapshot | null = null;

function applyPendingStreamEndViewport() {
  const snapshot = pendingStreamEndViewport;
  if (!snapshot || snapshot.sessionId !== props.activeSessionId) return;
  if (toolHandoffViewportQuiet.value || isSessionRestoreViewportGuardActive()) return;

  if (snapshot.followBottom) {
    scrollToBottomNow(true);
  } else {
    restoreMessagesScrollState(snapshot.state, snapshot.sessionId);
  }
  if (Date.now() >= snapshot.settleAfter) {
    pendingStreamEndViewport = null;
  }
}

const streamEndScrollScheduler = createSettledScrollScheduler(
  () => nextTick(applyPendingStreamEndViewport),
  STREAM_END_SCROLL_SETTLE_MS,
);

function handleToolHandoffQuietChange(quiet: boolean) {
  logToolCollapseTrace("chat-view", "toolHandoffQuietChange", {
    quiet,
    displayedStreamingLen: displayedStreamingText.value.length,
    isStreaming: props.isStreaming,
  });
  toolHandoffViewportQuiet.value = quiet;
}

watch(toolHandoffViewportQuiet, (quiet, previousQuiet) => {
  if (quiet) {
    scrollToBottomScheduler.cancel();
    preserveScrollAnchorScheduler.cancel();
    if (!pendingStreamEndViewport) streamEndScrollScheduler.cancel();
    return;
  }
  if (previousQuiet) {
    if (pendingStreamEndViewport) {
      streamEndScrollScheduler.schedule();
    } else {
      reconcileViewport();
    }
  }
});

function reconcileViewport(forceBottom = false) {
  if (toolHandoffViewportQuiet.value) {
    return;
  }
  if (pendingRestoreSessionId.value && pendingRestoreSessionId.value === props.activeSessionId) {
    restorePendingSessionScroll();
    return;
  }
  if (sessionRestoreLayoutStabilizing.value) {
    return;
  }
  if (restoreToolViewportAnchor()) {
    return;
  }

  const el = getMessagesElement();
  if (!el) return;

  const remembered = props.activeSessionId ? getSessionScrollState(props.activeSessionId) : null;
  if (shouldAutoScrollToBottom({ force: forceBottom, metrics: readMessageMetrics(el), remembered })) {
    scrollToBottom(forceBottom);
    return;
  }

  preserveScrollAnchor();
}

function reconcileStreamingLayoutNow() {
  if (toolHandoffViewportQuiet.value || isSessionRestoreViewportGuardActive()) return;
  if (restoreToolViewportAnchor()) return;

  const el = getMessagesElement();
  if (!el) return;
  const remembered = props.activeSessionId
    ? getSessionScrollState(props.activeSessionId)
    : null;
  if (!shouldAutoScrollToBottom({ metrics: readMessageMetrics(el), remembered })) return;

  scrollToBottomScheduler.cancel();
  scrollToBottomNow();
}

function settleStreamEndScroll() {
  const el = getMessagesElement();
  if (!el) return;

  const metrics = readMessageMetrics(el);
  const remembered = props.activeSessionId ? getSessionScrollState(props.activeSessionId) : null;
  const followBottom = shouldAutoScrollToBottom({ metrics, remembered });
  pendingStreamEndViewport = {
    sessionId: props.activeSessionId,
    followBottom,
    state: followBottom ? { mode: "bottom" } : captureCurrentSessionScrollState(el),
    settleAfter: Date.now() + STREAM_END_SCROLL_SETTLE_MS,
  };
  streamEndScrollScheduler.schedule();
}

function finishPendingSessionRestore(targetSessionId: string) {
  if (pendingRestoreSessionId.value !== targetSessionId || props.activeSessionId !== targetSessionId) return;
  pendingRestoreSessionId.value = null;
  pendingRestoreMessagesRef.value = null;
}

function scheduleSessionRestoreFollowup(
  targetSessionId: string,
  state: SessionScrollState | null,
) {
  cancelSessionRestoreFrame();
  sessionRestoreFrame = requestViewportFrame(() => {
    sessionRestoreFrame = 0;
    if (props.activeSessionId !== targetSessionId) {
      cancelSessionRestoreLayoutStabilization();
      return;
    }

    restoreMessagesScrollState(state, targetSessionId);
    finishSessionRestoreLayoutStabilization({
      targetSessionId,
      state,
    });
  });
}

function restorePendingSessionScroll(options: { defer?: boolean } = {}) {
  const targetSessionId = pendingRestoreSessionId.value;
  if (!targetSessionId || targetSessionId !== props.activeSessionId) {
    return;
  }
  if (isPendingSessionRestoreAwaitingMessages()) {
    return;
  }

  const restore = () => {
    const el = getMessagesElement();
    if (!el || pendingRestoreSessionId.value !== props.activeSessionId) {
      return;
    }

    const remembered = resolvePendingSessionRestoreState(getSessionScrollState(targetSessionId));
    restoreMessagesScrollState(remembered, targetSessionId);
    finishPendingSessionRestore(targetSessionId);
    scheduleSessionRestoreFollowup(targetSessionId, remembered);
  };

  if (options.defer) {
    nextTick(restore);
    return;
  }

  restore();
}

let olderHistoryRestoreRunning = false;

async function loadOlderHistoryAtTop(el: HTMLElement) {
  const historyLoading = props.scopedSession
    ? !!props.sessionHistoryLoading
    : chatStore.sessionHistoryLoading;
  const historyHasMore = props.scopedSession
    ? !!props.sessionHistoryHasMore
    : chatStore.sessionHistoryHasMore;
  const loadOlder = props.scopedSession
    ? props.loadOlderHistory
    : () => chatStore.loadOlderSessionHistory();
  if (
    olderHistoryRestoreRunning
    || historyLoading
    || !historyHasMore
    || !loadOlder
    || el.scrollTop > 160
  ) {
    return;
  }
  olderHistoryRestoreRunning = true;
  const sessionId = props.activeSessionId;
  const anchor = captureScrollAnchor(el);
  const previousScrollHeight = el.scrollHeight;
  try {
    const loaded = await loadOlder();
    if (!loaded || props.activeSessionId !== sessionId) return;
    await nextTick();
    if (!anchor) {
      suppressScrollCapture = true;
      el.scrollTop += Math.max(0, el.scrollHeight - previousScrollHeight);
      await new Promise<void>((resolve) => requestViewportFrame(resolve));
    } else {
      let stableFrameCount = 0;
      for (let frameCount = 0; frameCount < SESSION_RESTORE_MAX_SETTLE_FRAMES; frameCount += 1) {
        if (props.activeSessionId !== sessionId) return;
        suppressScrollCapture = true;
        const anchorRestored = restoreScrollAnchor(el, {
          mode: "anchor",
          anchorId: anchor.anchorId,
          offsetTop: anchor.offsetTop,
          fallbackScrollTop: anchor.fallbackScrollTop,
        });
        if (!anchorRestored) {
          el.scrollTop += Math.max(0, el.scrollHeight - previousScrollHeight);
          await new Promise<void>((resolve) => requestViewportFrame(resolve));
          break;
        }
        await new Promise<void>((resolve) => requestViewportFrame(resolve));
        const currentAnchor = Array.from(
          el.querySelectorAll<HTMLElement>("[data-scroll-anchor-id]"),
        ).find((candidate) => candidate.dataset.scrollAnchorId === anchor.anchorId);
        const offset = currentAnchor
          ? currentAnchor.getBoundingClientRect().top - el.getBoundingClientRect().top
          : Number.POSITIVE_INFINITY;
        stableFrameCount = Math.abs(offset - anchor.offsetTop) <= 1
          ? stableFrameCount + 1
          : 0;
        if (stableFrameCount >= SESSION_RESTORE_REQUIRED_STABLE_FRAMES) break;
      }
    }
    suppressScrollCapture = false;
    if (props.activeSessionId === sessionId) {
      rememberScrollForSession(sessionId);
    }
  } finally {
    suppressScrollCapture = false;
    olderHistoryRestoreRunning = false;
  }
}

function requestOlderHistoryAtTop() {
  if (turnNavigationRevealActive.value) return;
  const el = getMessagesElement();
  if (el && el.scrollTop <= 160) {
    void loadOlderHistoryAtTop(el);
  }
}

function handleTurnNavigationRevealState(active: boolean, messageId: string) {
  turnNavigationRevealActive.value = active;
  if (active) {
    markMessagesUserScrollIntent();
    return;
  }
  if (props.activeSessionId && props.messages.some((message) => message.id === messageId)) {
    const el = getMessagesElement();
    const anchor = el?.querySelector<HTMLElement>(
      `[data-scroll-anchor-id="${CSS.escape(messageId)}"]`,
    );
    if (el && anchor) {
      rememberSessionScrollState(props.activeSessionId, {
        mode: "anchor",
        anchorId: messageId,
        offsetTop: anchor.getBoundingClientRect().top - el.getBoundingClientRect().top,
        fallbackScrollTop: el.scrollTop,
      });
    }
  }
}

function preserveMessagesViewportForUserScroll() {
  cancelSessionRestoreFrame();
  cancelSessionRestoreLayoutStabilization();
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  pendingStreamEndViewport = null;
  streamEndScrollScheduler.cancel();
  rememberScrollForSession();
}

function onMessagesScroll() {
  if (suppressScrollCapture) {
    recordLayoutDiagnostic("chat.sessionScroll.scrollEventSuppressed", {
      sessionId: props.activeSessionId ?? null,
      metrics: readSessionScrollMetrics(),
    });
    return;
  }
  if (isSessionRestoreViewportGuardActive()) {
    recordLayoutDiagnostic("chat.sessionScroll.scrollEventSkippedDuringRestore", {
      sessionId: props.activeSessionId ?? null,
      pendingRestoreSessionId: pendingRestoreSessionId.value,
      sessionRestoreLayoutStabilizing: sessionRestoreLayoutStabilizing.value,
      metrics: readSessionScrollMetrics(),
    });
    return;
  }
  // Pagination follows the viewport position itself. A long scrollbar drag can
  // outlive the user-intent TTL, and an upward wheel at scrollTop=0 produces no
  // additional scroll event, so either path must be able to request the page.
  requestOlderHistoryAtTop();
  if (!userScrollIntent.isRecent()) {
    const el = getMessagesElement();
    if (props.activeSessionId && el && isNearBottom(readMessageMetrics(el))) {
      rememberSessionScrollState(props.activeSessionId, { mode: "bottom" });
    }
    recordLayoutDiagnostic("chat.sessionScroll.nonUserScrollIgnored", {
      sessionId: props.activeSessionId ?? null,
      state: props.activeSessionId ? getSessionScrollState(props.activeSessionId) : null,
      metrics: readSessionScrollMetrics(),
    });
    return;
  }

  preserveMessagesViewportForUserScroll();
  recordLayoutDiagnostic("chat.sessionScroll.userScrollCaptured", {
    sessionId: props.activeSessionId ?? null,
    state: props.activeSessionId ? getSessionScrollState(props.activeSessionId) : null,
    metrics: readSessionScrollMetrics(),
  });
}

let transcriptResizeObserver: ResizeObserverHandle | null = null;
let transcriptResizeFrame = 0;
let transcriptResizeReconcilePending = false;
let transcriptObservedViewportWidth = 0;

function readTranscriptViewportWidth() {
  if (typeof window === "undefined") return 0;
  return Math.max(1, Math.round(window.innerWidth || document.documentElement?.clientWidth || 0));
}

function isLiveResizeInProgress() {
  return uiStore.isWindowResizing || isDraggingSession.value || uiStore.isAssistantSidebarTransitioning;
}

function noteTranscriptViewportResize() {
  const width = readTranscriptViewportWidth();
  if (!width) return false;
  const previousWidth = transcriptObservedViewportWidth;
  transcriptObservedViewportWidth = width;
  if (previousWidth > 0 && Math.abs(width - previousWidth) >= 1) {
    recordLayoutDiagnostic("chat.transcript.viewportResize", { width, previousWidth });
    return true;
  }
  return false;
}

function cancelTranscriptResizeReconcileFrame() {
  if (!transcriptResizeFrame) return;
  cancelViewportFrame(transcriptResizeFrame);
  transcriptResizeFrame = 0;
}

function performTranscriptResizeReconcile() {
  if (suppressScrollCapture || toolHandoffViewportQuiet.value) {
    return;
  }
  if (pendingRestoreSessionId.value && pendingRestoreSessionId.value === props.activeSessionId) {
    restorePendingSessionScroll();
    return;
  }
  if (sessionRestoreLayoutStabilizing.value) {
    return;
  }
  if (restoreToolViewportAnchor()) {
    return;
  }
  reconcileViewport();
}

function scheduleTranscriptResizeReconcile(reason: string) {
  transcriptResizeReconcilePending = true;
  if (transcriptResizeFrame) return;

  transcriptResizeFrame = requestViewportFrame(() => {
    transcriptResizeFrame = 0;
    if (!transcriptResizeReconcilePending) return;
    if (isLiveResizeInProgress()) {
      recordLayoutDiagnostic("chat.transcript.resize.deferred", {
        reason,
        windowResizing: uiStore.isWindowResizing,
        sessionDragging: isDraggingSession.value,
        sidebarTransitioning: uiStore.isAssistantSidebarTransitioning,
      });
      return;
    }

    transcriptResizeReconcilePending = false;
    recordLayoutDiagnostic("chat.transcript.resize.reconcile", { reason });
    performTranscriptResizeReconcile();
  });
}

function handleTranscriptResize() {
  const viewportResizing = noteTranscriptViewportResize();
  if (viewportResizing || isLiveResizeInProgress()) {
    transcriptResizeReconcilePending = true;
    recordLayoutDiagnostic("chat.transcript.resize.defer", {
      windowResizing: uiStore.isWindowResizing,
      sessionDragging: isDraggingSession.value,
      sidebarTransitioning: uiStore.isAssistantSidebarTransitioning,
      viewportResizing,
    });
    return;
  }

  scheduleTranscriptResizeReconcile("observer");
}

function flushPendingTranscriptResizeReconcile(reason: string) {
  if (!transcriptResizeReconcilePending) return;
  scheduleTranscriptResizeReconcile(reason);
}

function disconnectTranscriptResizeObserver() {
  cancelTranscriptResizeReconcileFrame();
  transcriptResizeReconcilePending = false;
  transcriptResizeObserver?.disconnect();
  transcriptResizeObserver = null;
}

function connectTranscriptResizeObserver() {
  disconnectTranscriptResizeObserver();
  if (typeof ResizeObserver === "undefined") return;

  const scrollEl = getMessagesElement();
  const contentEl = getMessagesContentElement();
  if (!scrollEl && !contentEl) return;
  transcriptObservedViewportWidth = readTranscriptViewportWidth();

  transcriptResizeObserver = createAnimationFrameResizeObserver(handleTranscriptResize);
  if (!transcriptResizeObserver) return;

  if (scrollEl) {
    transcriptResizeObserver.observe(scrollEl);
  }
  if (contentEl && contentEl !== scrollEl) {
    transcriptResizeObserver.observe(contentEl);
  }
}

watch(
  () => props.activeSessionId,
  (nextSessionId, previousSessionId) => {
    if (nextSessionId) {
      beginSessionRestoreLayoutStabilization();
    } else {
      cancelSessionRestoreLayoutStabilization();
    }
    clearToolViewportAnchor();
    scrollToBottomScheduler.cancel();
    pendingStreamEndViewport = null;
    streamEndScrollScheduler.cancel();
    preserveScrollAnchorScheduler.cancel();
    cancelSessionRestoreFrame();
    toolHandoffViewportQuiet.value = false;
    if (previousSessionId) {
      rememberScrollForSession(previousSessionId);
    }

    const shouldRestoreImmediately = !!nextSessionId && previousSessionId === null && !showWelcomeState.value;
    pendingRestoreSessionId.value = nextSessionId;
    pendingRestoreMessagesRef.value = nextSessionId && !shouldRestoreImmediately ? props.messages : null;
    void restoreComposerDraft(nextSessionId ?? null);
    if (shouldRestoreImmediately) {
      restorePendingSessionScroll({ defer: true });
    } else if (nextSessionId) {
      // Pinia commits the active id and the ready message page in one update.
      // Vue can therefore deliver both child props in the same render, leaving
      // no later message identity change to release the restore sentinel.
      nextTick(() => {
        if (pendingRestoreSessionId.value !== nextSessionId) return;
        pendingRestoreMessagesRef.value = null;
        restorePendingSessionScroll();
      });
    }
  },
  { flush: "sync" },
);

watch(
  () => props.messages,
  (messages) => {
    if (!pendingRestoreSessionId.value || pendingRestoreSessionId.value !== props.activeSessionId) return;
    if (messages === pendingRestoreMessagesRef.value) return;
    restorePendingSessionScroll();
  },
  { flush: "post" },
);

watch(
  () => props.messages,
  (messages, previous) => {
    if (messages === previous || pendingRestoreSessionId.value) return;
    reconcileViewport();
  },
  { flush: "post" },
);
watch(
  () => props.messages.length,
  () => {
    reconcileViewport();
  },
  { flush: "post" },
);
watch(() => displayedStreamingText.value, () => reconcileViewport());
// Reconcile on tool-call identity/status changes only. A deep watch here would
// re-traverse every tool call (including large streamed output strings) and
// force a synchronous layout read on each output delta; growth-driven height
// changes are already covered by the transcript ResizeObserver.
watch(
  () => props.activeToolCalls.map((toolCall) => `${toolCall.id}:${toolCall.status}`).join(","),
  () => reconcileViewport(),
);
watch(
  () => props.isStreaming,
  (nextStreaming, previousStreaming) => {
    logToolCollapseTrace("chat-view", "isStreamingChanged", {
      previous: previousStreaming,
      next: nextStreaming,
      displayedStreamingLen: displayedStreamingText.value.length,
      sourceStreamingLen: props.streamingText.length,
      activeToolCallCount: props.activeToolCalls.length,
      quiet: toolHandoffViewportQuiet.value,
    });
    if (nextStreaming) {
      pendingStreamEndViewport = null;
      streamEndScrollScheduler.cancel();
      return;
    }
    if (previousStreaming) {
      settleStreamEndScroll();
    }
  },
);
watch(isWaitingForResponse, (v) => { if (v) reconcileViewport(); });
watch(() => props.pendingQuestion?.questionId ?? null, (q) => {
  if (q) reconcileViewport();
});
watch(() => props.pendingToolConfirms.map((item) => item.questionId).join(":"), (value) => {
  if (value) reconcileViewport();
});

const keepBatchToolConfirmLayout = ref(false);

watch(
  () => [props.activeSessionId, props.pendingToolConfirms.map((item) => item.questionId).join(":")],
  ([sessionId], previous = []) => {
    const [prevSessionId] = previous;
    const count = props.pendingToolConfirms.length;
    if (sessionId !== prevSessionId) {
      keepBatchToolConfirmLayout.value = count > 1;
      return;
    }
    if (count === 0) {
      keepBatchToolConfirmLayout.value = false;
      return;
    }
    if (count > 1) {
      keepBatchToolConfirmLayout.value = true;
    }
  },
  { immediate: true },
);

const showBatchToolConfirmCard = computed(() =>
  !isViewingSubagent.value
  && props.pendingToolConfirms.length > 0
  && (keepBatchToolConfirmLayout.value || props.pendingToolConfirms.length > 1),
);

const showSingleToolConfirmCard = computed(() =>
  !isViewingSubagent.value
  && props.pendingToolConfirms.length === 1
  && !showBatchToolConfirmCard.value,
);

// Standalone plan review window: auto-open when the user prefers the window
// surface, and tell an open window when the pending approval settles (the
// card answered it, the run was cancelled, ...) so it drops its action bar.
watch(
  () => props.pendingToolConfirms.find((item) => item.display.kind === "planApproval")?.questionId ?? "",
  (questionId, previousQuestionId) => {
    if (questionId) {
      if (questionId === previousQuestionId) return;
      if (displaySettings.planApprovalTarget !== "window") return;
      const confirm = props.pendingToolConfirms.find((item) => item.questionId === questionId);
      if (confirm?.display.kind === "planApproval" && props.activeSessionId) {
        void openPlanViewWindow({
          sessionId: props.activeSessionId,
          planFilePath: confirm.display.planFilePath,
          questionId,
        });
      }
      return;
    }
    if (previousQuestionId) {
      void broadcastPlanApprovalResolved(previousQuestionId);
    }
  },
  { immediate: true },
);

async function handleRequestPlanMode(active: boolean) {
  if (props.scopedSession) {
    emit("requestPlanMode", active);
    return;
  }
  if (!props.activeSessionId) return;
  try {
    await chatStore.setSessionPlanMode(props.activeSessionId, active);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", t("chat.plan.exitFailed"), {
      code: err.code,
      operation: "plan-mode-exit",
    });
  }
}

async function handleExitPlanMode() {
  if (props.scopedSession) {
    if (!props.exitPlanMode) return;
    try {
      await props.exitPlanMode();
    } catch (e) {
      const err = normalizeAppError(e);
      notificationStore.addNotice("error", t("chat.plan.exitFailed"), {
        code: err.code,
        operation: "plan-mode-exit",
      });
    }
    return;
  }
  await handleRequestPlanMode(false);
}

function handleComposerSend(payload: ChatComposerSendPayload) {
  emit("send", payload.text, payload.images, payload.assetRefs, {
    displayText: payload.displayText,
    mode: payload.mode ?? undefined,
    userIntent: payload.userIntent ?? null,
  });
}

const STORAGE_KEY_SESSION_WIDTH = "locus:sessionPanelWidth";
const STORAGE_KEY_SESSION_COLLAPSED = "locus:sessionPanelCollapsed";
const sessionPanelWidth = ref(220); // px
const sessionPanelCollapsed = ref(!!props.defaultSessionPanelCollapsed);
const isDraggingSession = ref(false);
const layoutRef = ref<HTMLElement | null>(null);
let releaseSessionSelectionLock: (() => void) | null = null;
let sessionSplitterLayoutLeft = 0;
let pendingSessionPanelWidth: number | null = null;
let sessionSplitterFrame = 0;

const sessionPanelWidthStorageKey = computed(() =>
  props.sessionPanelStorageScope
    ? `locus:${props.sessionPanelStorageScope}:sessionPanelWidth`
    : STORAGE_KEY_SESSION_WIDTH,
);
const sessionPanelCollapsedStorageKey = computed(() =>
  props.sessionPanelStorageScope
    ? `locus:${props.sessionPanelStorageScope}:sessionPanelCollapsed`
    : STORAGE_KEY_SESSION_COLLAPSED,
);

const resolvedLayoutMode = computed<ResolvedChatLayoutMode>(() => {
  if (props.layoutMode === "vertical") return "vertical";
  return "horizontal";
});
const isVerticalLayout = computed(() => resolvedLayoutMode.value === "vertical");
const showSessionPanel = computed(() =>
  props.showSessionNavigation !== false
  && !showInlineDiff.value
  && !isVerticalLayout.value
  && !sessionPanelCollapsed.value,
);
const showSessionCompactPicker = computed(() =>
  props.showSessionNavigation !== false
  && (isVerticalLayout.value || sessionPanelCollapsed.value),
);

watch(
  resolvedLayoutMode,
  (mode) => emit("layoutModeChange", mode),
  { immediate: true },
);

watch(
  () => uiStore.isWindowResizing,
  (resizing) => {
    if (resizing) return;
    transcriptObservedViewportWidth = readTranscriptViewportWidth();
    flushPendingTranscriptResizeReconcile("window-resize-settled");
  },
);

watch(isDraggingSession, (dragging) => {
  if (dragging) return;
  flushPendingTranscriptResizeReconcile("session-drag-settled");
});

watch(
  () => uiStore.isAssistantSidebarTransitioning,
  (transitioning) => {
    if (transitioning) return;
    flushPendingTranscriptResizeReconcile("sidebar-transition-settled");
  },
);

function clampSessionPanelWidth(width: number) {
  return Math.max(140, Math.min(480, Math.round(width)));
}

function commitSessionPanelWidth(width: number) {
  const nextWidth = clampSessionPanelWidth(width);
  if (sessionPanelWidth.value === nextWidth) return;
  sessionPanelWidth.value = nextWidth;
}

function cancelSessionSplitterFrame() {
  if (!sessionSplitterFrame) return;
  cancelViewportFrame(sessionSplitterFrame);
  sessionSplitterFrame = 0;
}

function flushSessionSplitterWidth() {
  cancelSessionSplitterFrame();
  if (pendingSessionPanelWidth === null) return;
  commitSessionPanelWidth(pendingSessionPanelWidth);
  recordLayoutDiagnostic("chat.sessionSplitter.width", {
    width: clampSessionPanelWidth(pendingSessionPanelWidth),
  });
  pendingSessionPanelWidth = null;
}

function scheduleSessionPanelWidth(width: number) {
  pendingSessionPanelWidth = width;
  if (sessionSplitterFrame) return;
  sessionSplitterFrame = requestViewportFrame(flushSessionSplitterWidth);
}

function onSessionSplitterMouseDown(e: MouseEvent) {
  e.preventDefault();
  if (isVerticalLayout.value || sessionPanelCollapsed.value) return;
  sessionSplitterLayoutLeft = layoutRef.value?.getBoundingClientRect().left ?? 0;
  isDraggingSession.value = true;
  releaseSessionSelectionLock?.();
  releaseSessionSelectionLock = acquireSelectionLock();
  document.addEventListener("mousemove", onSessionSplitterMouseMove);
  document.addEventListener("mouseup", onSessionSplitterMouseUp);
}

function onSessionSplitterMouseMove(e: MouseEvent) {
  if (!isDraggingSession.value) return;
  const x = e.clientX - sessionSplitterLayoutLeft;
  scheduleSessionPanelWidth(x);
}

function onSessionSplitterMouseUp() {
  flushSessionSplitterWidth();
  isDraggingSession.value = false;
  document.removeEventListener("mousemove", onSessionSplitterMouseMove);
  document.removeEventListener("mouseup", onSessionSplitterMouseUp);
  releaseSessionSelectionLock?.();
  releaseSessionSelectionLock = null;
  try { localStorage.setItem(sessionPanelWidthStorageKey.value, String(sessionPanelWidth.value)); } catch {}
}

function setSessionPanelCollapsed(value: boolean) {
  sessionPanelCollapsed.value = value;
  try { localStorage.setItem(sessionPanelCollapsedStorageKey.value, value ? "1" : "0"); } catch {}
}

const CANCEL_SHORTCUT_CONFIRM_OPERATION = "chat.cancelShortcutConfirm";
const cancelShortcutTracker = createDoublePressShortcutTracker();

function clearCancelShortcutConfirmation() {
  cancelShortcutTracker.reset();
  notificationStore.clearByOperation(CANCEL_SHORTCUT_CONFIRM_OPERATION);
}

function onGlobalChatKeydown(e: KeyboardEvent) {
  if (!props.scopedSession && uiStore.activeTab !== "chat") return;
  if (props.shortcutActive === false) return;

  if (e.key === "Escape" && showInlineDiff.value) {
    e.preventDefault();
    clearCancelShortcutConfirmation();
    chatChangesStore.closeInlineDiffForSession(props.activeSessionId);
    return;
  }

  if (e.defaultPrevented) {
    clearCancelShortcutConfirmation();
    return;
  }

  if (!e.repeat && props.isStreaming && matchesShortcut(e, shortcutState.cancelRun)) {
    e.preventDefault();
    if (props.isCancelling) {
      clearCancelShortcutConfirmation();
      return;
    }

    if (cancelShortcutTracker.press()) {
      clearCancelShortcutConfirmation();
      emit("cancel");
      return;
    }

    notificationStore.addNotice(
      "info",
      t("chat.cancelShortcut.confirm", formatShortcut(shortcutState.cancelRun)),
      {
        operation: CANCEL_SHORTCUT_CONFIRM_OPERATION,
        replaceOperation: true,
        ttl: DOUBLE_PRESS_SHORTCUT_INTERVAL_MS,
        skipConsoleLog: true,
      },
    );
    return;
  }

  if (!e.repeat && !["Control", "Meta", "Alt", "Shift"].includes(e.key)) {
    clearCancelShortcutConfirmation();
  }

  if (!e.repeat && matchesShortcut(e, shortcutState.newChat)) {
    e.preventDefault();
    handleNewChatRequest("shortcut");
    return;
  }
}

watch(
  () => [props.isStreaming, props.isCancelling] as const,
  ([isStreaming, isCancelling]) => {
    if (!isStreaming || isCancelling) {
      clearCancelShortcutConfirmation();
    }
  },
);

onMounted(() => {
  window.addEventListener("keydown", onGlobalChatKeydown);
  try {
    const saved = localStorage.getItem(sessionPanelWidthStorageKey.value);
    if (saved) sessionPanelWidth.value = Math.max(140, Math.min(480, Number(saved)));
  } catch {}
  try {
    const saved = localStorage.getItem(sessionPanelCollapsedStorageKey.value);
    sessionPanelCollapsed.value = saved === null
      ? !!props.defaultSessionPanelCollapsed
      : saved === "1";
  } catch {}
  nextTick(() => {
    connectTranscriptResizeObserver();
  });
});

onUnmounted(() => {
  window.removeEventListener("keydown", onGlobalChatKeydown);
  clearCancelShortcutConfirmation();
  rememberScrollForSession();
  clearInputControlsSwitchTimer();
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  pendingStreamEndViewport = null;
  streamEndScrollScheduler.cancel();
  cancelSessionRestoreFrame();
  cancelSessionRestoreLayoutStabilization();
  clearSessionRestoreLayoutTimer();
  clearToolViewportAnchor();
  clearStreamingTextFlushTimer();
  cancelSessionSplitterFrame();
  disconnectTranscriptResizeObserver();
  document.removeEventListener("mousemove", onSessionSplitterMouseMove);
  document.removeEventListener("mouseup", onSessionSplitterMouseUp);
  releaseSessionSelectionLock?.();
  releaseSessionSelectionLock = null;
});
</script>

<template>
  <div
    class="chat-view-layout"
    ref="layoutRef"
    :class="{
      'dragging-session': isDraggingSession,
      'is-vertical-layout': isVerticalLayout,
      'is-horizontal-layout': !isVerticalLayout,
    }"
  >

    <!-- Inline diff panel — covers entire chat layout (session panel + chat area) -->
    <div v-if="showInlineDiff" class="diff-inline-panel">
      <template v-if="inlineDiffPayload">
        <div class="diff-inline-header">
          <span class="diff-inline-status" :class="'status-' + (inlineDiffPayload.status ?? '').toLowerCase()">
            {{ inlineDiffPayload.status }}
          </span>
          <span v-if="inlineDiffPayload.oldPath" class="diff-inline-path" :title="inlineDiffPayload.oldPath + ' → ' + inlineDiffPayload.filePath">
            {{ inlineDiffPayload.oldPath }} → {{ inlineDiffPayload.filePath }}
          </span>
          <span v-else class="diff-inline-path" :title="inlineDiffPayload.filePath">
            {{ inlineDiffPayload.filePath }}
          </span>
          <BaseSegmented
            v-if="chatDiffViewerRef?.hasSemanticAndText"
            class="diff-inline-tab-group"
            size="sm"
            :model-value="chatDiffViewerRef.activeTab"
            :options="chatDiffTabOptions"
            @update:model-value="chatDiffViewerRef.activeTab = $event as 'semantic' | 'text'"
          />
          <span class="diff-inline-stats">
            <span class="stat-add">+{{ inlineDiffPayload.stats.additions }}</span>
            <span class="stat-del">-{{ inlineDiffPayload.stats.deletions }}</span>
          </span>
          <span class="diff-inline-actions">
            <BaseButton
              class="diff-inline-action-btn ui-select-none"
              :title="t('chat.changes.openReviewWindow')"
              @click="openInlineDiffInWindow"
            >
              <LucideIcon :icon="PanelTopOpen" :size="13" />
              {{ t('chat.changes.openReviewWindow') }}
            </BaseButton>
            <BaseButton
              v-if="!inlineDiffPayload.isBinary && canOpenInEditor(inlineDiffPayload.filePath)"
              class="diff-inline-action-btn ui-select-none"
              :title="t('common.openInEditor')"
              @click="openInlineDiffFileExternal"
            >
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
              {{ t('common.openInEditor') }}
            </BaseButton>
          </span>
          <button
            class="diff-close-btn ui-select-none"
            type="button"
            :title="t('common.close')"
            :aria-label="t('common.close')"
            @click="chatChangesStore.closeInlineDiffForSession(activeSessionId)"
          >&times;</button>
        </div>
        <div class="diff-inline-body">
          <FileDiffViewer
            ref="chatDiffViewerRef"
            :payload="inlineDiffPayload"
            :hide-builtin-tabs="true"
            @lfs-pulled="onChatDiffLfsPulled"
          />
        </div>
      </template>
      <div v-else-if="inlineDiffLoading" class="diff-inline-loading">
        <div class="diff-progress-info">
          <span class="diff-progress-text">{{ diffProgress.phaseLabel }}</span>
          <div class="diff-progress-bar">
            <div class="diff-progress-fill" :style="{ width: diffProgressWidth }"></div>
          </div>
        </div>
      </div>
      <div v-else-if="inlineDiffError" class="diff-inline-error">{{ inlineDiffError }}</div>
    </div>

    <SessionPanel
      v-if="showSessionNavigation !== false"
      v-show="showSessionPanel"
      :sessions="sessions"
      :active-session-id="activeSessionId"
      :pending-session-id="pendingSessionId"
      :streaming-session-ids="streamingSessionIds"
      :session-panel-width="sessionPanelWidth"
      :working-dir="workingDir"
      :show-views="displaySettings.showViewsInSessionPanel"
      @select-session="emit('selectSession', $event)"
      @new-chat="handleNewChatRequest"
      @rename-session="(id: string, title: string) => emit('renameSession', id, title)"
      @archive-session="emit('archiveSession', $event)"
      @delete-session="emit('deleteSession', $event)"
      @export-session-context="emit('exportSessionContext', $event)"
      @review-session-context="emit('reviewSessionContext', $event)"
      @toggle-panel-collapsed="setSessionPanelCollapsed(true)"
    />

    <div
      v-if="showSessionNavigation !== false"
      v-show="showSessionPanel"
      class="session-divider"
      @mousedown="onSessionSplitterMouseDown"
    ></div>

    <div
      v-show="!showInlineDiff"
      class="chat-view"
      :class="{ 'is-vertical-layout': isVerticalLayout }"
      :style="chatContentStyle"
    >
      <SessionCompactPicker
        v-if="showSessionCompactPicker"
        :sessions="sessions"
        :active-session-id="activeSessionId"
        :pending-session-id="pendingSessionId"
        :streaming-session-ids="streamingSessionIds"
        :show-expand-panel-button="sessionPanelCollapsed && !isVerticalLayout"
        :working-dir="workingDir"
        :show-views="displaySettings.showViewsInSessionPanel"
        @select-session="emit('selectSession', $event)"
        @new-chat="handleNewChatRequest"
        @expand-panel="setSessionPanelCollapsed(false)"
      />
      <div class="chat-main">
        <ChatTranscript
          ref="transcriptRef"
          variant="session"
          :class="{ 'is-session-restore-stabilizing': sessionRestoreLayoutStabilizing }"
          :session-key="sessionSurfaceKey"
          :workspace-ref="workspaceRef"
          :messages="messages"
          :streaming-text="displayedStreamingText"
          :typed-text-stream="typedTextStream"
          :has-streaming-text="hasStreamingText"
          :streaming-text-order="streamingTextOrder"
          :is-streaming="isStreaming"
          :is-compacting="isCompacting"
          :is-thinking="isThinking"
          :has-thinking="hasThinking"
          :thinking-order="thinkingOrder"
          :thinking-duration="thinkingDuration"
          :live-render-parts="liveRenderParts"
          :live-part-streams="livePartStreams"
          :active-tool-calls="activeToolCalls"
          :selected-message-id="contextSelectedMessageId"
          user-label="You"
          assistant-label="Locus"
          :handoff-label="t('chat.transcript.handoff')"
          :waiting-label="t('chat.transcript.waiting')"
          :compacting-label="t('chat.transcript.compacting')"
          :compacted-label="t('chat.transcript.compacted')"
          :compacted-context-open-label="t('chat.transcript.openCompactedContext')"
          :enable-compacted-context-open="!!activeSessionId"
          :thinking-active-label="t('chat.transcript.thinking')"
          :thought-duration-label="t('chat.transcript.thoughtDuration', '{0}')"
          :thought-moment-label="t('chat.transcript.thoughtMoment')"
          enable-intent-badges
          show-user-images
          user-content-mode="asset"
          @scroll="onMessagesScroll"
          @user-scroll-intent="markMessagesUserScrollIntent"
          @content-click="handleContentClick"
          @content-contextmenu="handleContentContextMenu"
          @content-pointerdown="handleContentPointerDown"
          @open-thinking="emit('openThinking', $event)"
          @open-image="openLightbox"
          @open-compacted-context="openCompactedContext"
          @apply-knowledge-proposal="handleApplyKnowledgeProposal"
          @ignore-knowledge-proposal="handleIgnoreKnowledgeProposal"
          @tool-handoff-quiet-change="handleToolHandoffQuietChange"
          @tool-viewport-anchor-start="handleToolViewportAnchorStart"
          @tool-viewport-anchor-end="handleToolViewportAnchorEnd"
          @stream-layout-change="reconcileStreamingLayoutNow"
        >
        </ChatTranscript>
        <ChatTurnNavigationRail
          v-if="displaySettings.showTurnNavigationRail"
          :messages="messages"
          :session-id="activeSessionId"
          :user-message-ids="turnNavigationUserMessageIds"
          :scroll-element="transcriptScrollElement"
          :load-preview="turnNavigationLoadPreview"
          :load-turn="turnNavigationLoadTurn"
          @navigate="markMessagesUserScrollIntent"
          @reveal-state="handleTurnNavigationRevealState"
        />
        <div v-if="showWelcomeState" class="chat-empty-overlay">
          <div class="empty-state">
            <div class="empty-icon">L</div>
            <div class="empty-title">Locus</div>
            <div v-if="displaySettings.showWelcomeSubtitle" class="empty-subtitle">{{ t("onboarding.welcome.subtitle") }}</div>
          </div>
        </div>
      </div>

    <div
      v-if="(pendingQuestion && !isViewingSubagent) || showBatchToolConfirmCard || showSingleToolConfirmCard || showQueuedFollowUp || showCompactQueued || (planModeActive && !isViewingSubagent)"
      class="chat-pending-stack"
      @wheel="handleBottomPanelWheel"
    >
      <div v-if="showQueuedFollowUp" class="queued-follow-up-bar">
        <span class="queued-follow-up-label">
          {{ activeQueuedFollowUp?.isInserting ? t('chat.input.queuedFollowUpInserting') : t('chat.input.queuedFollowUp') }}
        </span>
        <QueuedFollowUpImages :images="activeQueuedFollowUp?.images" />
        <span class="queued-follow-up-text ui-select-text">{{ activeQueuedFollowUp?.displayText }}</span>
        <BaseButton
          v-if="activeQueuedFollowUp?.canInsert"
          class="queued-follow-up-insert"
          size="sm"
          variant="neutral"
          type="button"
          @click="handleInsertQueuedFollowUp"
        >
          {{ t('chat.input.queuedFollowUpInsert') }}
        </BaseButton>
        <BaseButton
          class="queued-follow-up-re-edit"
          size="sm"
          variant="neutral"
          type="button"
          @click="handleReEditQueuedFollowUp"
        >
          {{ t('chat.messageMenu.reEditUserMessage') }}
        </BaseButton>
        <BaseButton
          class="queued-follow-up-delete"
          size="sm"
          variant="neutral"
          type="button"
          @click="handleDeleteQueuedFollowUp"
        >
          {{ t('common.delete') }}
        </BaseButton>
      </div>
      <div v-if="showCompactQueued" class="queued-follow-up-bar">
        <span class="queued-follow-up-label">{{ t('chat.input.compactQueued') }}</span>
      </div>

      <AskUserCard
        v-if="pendingQuestion && !isViewingSubagent"
        :question="pendingQuestion"
        @answer="handleQuestionAnswer"
      />

      <ToolConfirmBatchCard
        v-if="showBatchToolConfirmCard"
        :tool-confirms="pendingToolConfirms"
        :session-id="activeSessionId"
        @answer="emit('answerToolConfirm', $event.questionId, $event.answer)"
        @answer-many="emit('answerAllToolConfirms', $event.questionIds, $event.answer)"
      />
      <ToolConfirmCard
        v-else-if="showSingleToolConfirmCard"
        :tool-confirm="pendingToolConfirms[0]!"
        :session-id="activeSessionId"
        @answer="emit('answerToolConfirm', pendingToolConfirms[0]!.questionId, $event)"
      />

      <!-- Sticky plan mode status bar: visible until exit_plan_mode is
           approved or the user exits manually. -->
      <div v-if="planModeActive && !isViewingSubagent" class="plan-status-bar">
        <svg class="plan-status-icon" viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
          <path d="M2 3.5A1.5 1.5 0 0 1 3.5 2h9A1.5 1.5 0 0 1 14 3.5v9a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5v-9zM5 5h6v1H5V5zm0 3h6v1H5V8zm0 3h4v1H5v-1z"/>
        </svg>
        <span>{{ t('chat.plan.stickyActive') }}</span>
        <button
          class="plan-status-exit ui-select-none"
          type="button"
          :disabled="props.isStreaming || (scopedSession && !exitPlanMode)"
          :title="t('chat.plan.exit')"
          @click="handleExitPlanMode"
        >
          {{ t('chat.plan.exit') }}
        </button>
      </div>
    </div>

    <div
      class="input-area"
      :class="{
        'is-controls-collapsed': inputControlsCollapsed,
        'is-controls-switching': inputControlsSwitching,
        'is-reference-drop-available': referenceDropAvailable || referenceDropActive,
      }"
    >
      <div class="chat-input-frame">
        <div class="input-controls-toggle-zone">
          <button
            class="input-controls-toggle ui-select-none"
            :class="{ 'is-collapsed': inputControlsCollapsed }"
            type="button"
            :title="inputControlsToggleTitle"
            :aria-label="inputControlsToggleTitle"
            :aria-pressed="inputControlsCollapsed"
            @click="toggleInputControlsCollapsed"
          >
            <svg
              v-if="inputControlsCollapsed"
              class="input-controls-toggle-icon"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M4 10l4-4 4 4" />
            </svg>
            <svg
              v-else
              class="input-controls-toggle-icon"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M4 6l4 4 4-4" />
            </svg>
          </button>
        </div>
        <div v-if="!inputControlsCollapsed" class="input-backdrop-row">
          <div v-if="!inputControlsCollapsed" class="input-backdrop-status">
            <ChatStatusIndicators
              :unity-connected="unityConnected"
              :unity-plugin-status="unityPluginStatus"
              :unity-plugin-installing="unityPluginInstalling"
              :unity-launching="unityLaunching"
              :unity-launch-state="unityLaunchState"
              :unity-connection-status="unityConnectionStatus"
              :unity-recompiling="unityRecompileActive"
              :workspace-ref="workspaceRef"
              :project-services="projectServices"
              :working-dir="workingDir"
              :scan-phase="scanPhase"
              :last-scan-stats="lastScanStats"
              :knowledge-access-mode="knowledgeAccessMode"
              :selected-agent-id="selectedAgentId"
              @start-scan="emit('startScan')"
              @install-plugin="emit('installPlugin')"
              @launch-unity-project="emit('launchUnityProject')"
              @update-knowledge-access-mode="setKnowledgeAccessMode"
            />
          </div>
          <div class="input-backdrop-action">
            <button
              v-if="!isViewingSubagent && hasPanelToggleRow"
              class="changes-toggle-btn ui-select-none"
              :class="{ 'is-active': currentChangesPanelVisible }"
              type="button"
              :disabled="isStreaming"
              :aria-pressed="currentChangesPanelVisible"
              :aria-label="t('chat.changes.toggle')"
              @click="chatChangesStore.togglePanelForSession(activeSessionId)"
            >
              <LucideIcon :icon="FileDiff" :size="14" />
              <span class="changes-toggle-label">{{ t('chat.changes.toggle') }}</span>
            </button>
          </div>
        </div>
        <RichChatInput
          ref="composerPanelRef"
          v-model="inputText"
          :selected-agent-id="selectedAgentId"
          :skills="skills"
          :managed-local-files="managedLocalFiles"
          :placeholder="chatInputPlaceholder"
          :is-streaming="isStreaming"
          :cancelling="isCancelling"
          :can-resume="canResumeInterrupted"
          :allow-runtime-compact="!isViewingSubagent"
          :send-label="isStreaming ? runningSendLabel : t('common.send')"
          :cancel-label="t('common.cancel')"
          :resume-label="t('chat.input.resume')"
          :compact="inputControlsCollapsed"
          :asset-ref-sync-key="composerAssetRefSyncKey"
          :draft-state-key="composerDraftStateKey"
          :workspace-ref="workspaceRef"
          :workspace-root="workingDir"
          :plan-mode-active="planModeActive"
          :reference-drop-available="referenceDropAvailable"
          :reference-drop-active="referenceDropActive"
          :managed-native-drops="managedNativeDrops"
          @send="handleComposerSend"
          @compact="emit('compact')"
          @fork="emit('fork')"
          @undo="openUndoChooser"
          @export-context="emit('exportSessionContext', { sessionId: activeSessionId || '' })"
          @review-context="emit('reviewSessionContext', { sessionId: activeSessionId || '' })"
          @request-plan-mode="handleRequestPlanMode"
          @request-new-session="handleNewChatRequest"
          @focus="emit('composerFocus')"
          @remove-managed-local-file="emit('removeManagedComposerFile', $event)"
          @clear="handleNewChatRequest"
          @cancel="emit('cancel')"
          @resume="emit('resume')"
        >
          <template v-if="!inputControlsCollapsed" #footer-start>
            <ModelEffortSelector
              align="start"
              :agents="displaySettings.showAgentSelector ? agents : undefined"
              :selected-agent-id="selectedAgentId"
              :agent-locked="agentLocked"
              :models="models"
              :selected-id="selectedModelId"
              :effort="effort"
              :efforts="effortLevels"
              :effort-supported="effortSupported"
              :multi-agent-enabled="multiAgentEnabled"
              :fast-mode-enabled="fastModeEnabled"
              :fast-mode-available="fastModeAvailable"
              :disabled="isStreaming"
              @select-agent="emit('selectAgent', $event)"
              @select-model="emit('selectModel', $event)"
              @select-effort="emit('selectEffort', $event)"
              @select-multi-agent="emit('selectMultiAgent', $event)"
              @select-fast-mode="emit('selectFastMode', $event)"
            />
            <TokenUsageBar
              :token-usage="tokenUsage"
              :active-session-id="activeSessionId"
              :codex-connected="codexConnected"
              @open-context-stats="contextStatsOpen = true"
            />
          </template>
        </RichChatInput>
      </div>
    </div>
    </div><!-- /chat-view -->

    <Teleport to="body">
      <SessionContextUsageWindow
        v-if="contextStatsOpen && activeSessionId"
        :session-id="activeSessionId"
        :model-id="selectedModelId"
        :knowledge-mode="knowledgeAccessMode"
        :token-usage="tokenUsage"
        @close="contextStatsOpen = false"
      />
    </Teleport>

    <Teleport to="body">
      <Transition name="undo-chooser-fade">
        <div
          v-if="undoChooserVisible"
          ref="undoChooserRef"
          class="undo-chooser-backdrop"
          tabindex="-1"
          @click.self="closeUndoChooser"
          @keydown="handleUndoChooserKeydown"
        >
          <div class="undo-chooser" role="dialog" aria-modal="true" :aria-label="t('chat.undo.dialogTitle')">
            <div class="undo-chooser-header">
              <div class="undo-chooser-title">{{ t("chat.undo.dialogTitle") }}</div>
              <button
                type="button"
                class="undo-chooser-close ui-select-none"
                :disabled="undoChooserBusy"
                :aria-label="t('common.cancel')"
                @click="closeUndoChooser"
              >
                &times;
              </button>
            </div>
            <div v-if="!canUndoConversation" class="undo-chooser-empty">
              {{ t("chat.undo.noConversationRound") }}
            </div>
            <div v-else class="undo-chooser-actions">
              <button
                type="button"
                class="undo-chooser-action"
                :class="{ 'is-selected': selectedUndoChoice === 'conversation' }"
                :disabled="undoChooserBusy"
                :aria-pressed="selectedUndoChoice === 'conversation'"
                @focus="selectedUndoChoice = 'conversation'"
                @click="undoConversationOnly"
              >
                <span>{{ t("chat.undo.conversationOnly") }}</span>
                <span v-if="undoAction === 'conversation'" class="undo-chooser-action-state">
                  {{ t("chat.changes.undoing") }}
                </span>
              </button>
              <button
                type="button"
                class="undo-chooser-action"
                :class="{ 'is-selected': selectedUndoChoice === 'files' }"
                :disabled="undoChooserBusy || !canUndoFilesAndConversation"
                :aria-pressed="selectedUndoChoice === 'files'"
                @focus="selectedUndoChoice = 'files'"
                @click="undoFilesAndConversation"
              >
                <span>{{ t("chat.undo.filesAndConversation") }}</span>
                <span v-if="undoAction === 'files'" class="undo-chooser-action-state">
                  {{ t("chat.changes.undoing") }}
                </span>
              </button>
              <div
                v-if="canUndoFilesAndConversation && undoChooserDirtyFiles.length > 0"
                class="undo-chooser-warning"
              >
                <div>{{ t("chat.undo.dirtyWarning") }}</div>
                <div class="undo-chooser-warning-files">{{ undoChooserDirtyPreview }}</div>
              </div>
              <div v-if="!canUndoFilesAndConversation" class="undo-chooser-note">
                {{ t("chat.undo.noFileUndo") }}
              </div>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <BaseContextMenu
      v-if="messageCtxMenu"
      class="asset-ref-ctx-menu"
      :x="messageCtxMenu.x"
      :y="messageCtxMenu.y"
      :min-width="184"
      @close="closeMessageContextMenu"
    >
          <button
            v-if="messageCtxMenu.selectionText"
            type="button"
            class="asset-ref-ctx-item ui-select-none"
            role="menuitem"
            @click="doMessageCopySelection"
          >
            <LucideIcon :icon="Copy" :size="13" />
            {{ t("chat.messageMenu.copySelection") }}
          </button>
          <button
            v-else-if="messageCtxMenu.messageId"
            type="button"
            class="asset-ref-ctx-item ui-select-none"
            role="menuitem"
            :disabled="!messageContextCanCopy"
            @click="doMessageCopy"
          >
            <LucideIcon :icon="Copy" :size="13" />
            {{ t("chat.messageMenu.copyMessage") }}
          </button>
          <button
            v-if="messageContextShouldShowReEdit"
            type="button"
            class="asset-ref-ctx-item ui-select-none"
            role="menuitem"
            :disabled="!messageContextCanReEdit"
            @click="doMessageReEdit"
          >
            <LucideIcon :icon="PencilLine" :size="13" />
            {{ t("chat.messageMenu.reEditUserMessage") }}
          </button>
          <template v-if="messageCtxMenu.messageId">
            <div class="asset-ref-ctx-sep"></div>
            <button
              type="button"
              class="asset-ref-ctx-item ui-select-none"
              role="menuitem"
              :disabled="!messageContextCanAct"
              @click="doMessageRollback"
            >
              <LucideIcon :icon="Undo2" :size="13" />
              {{ t("chat.messageMenu.rollbackToMessage") }}
            </button>
            <button
              type="button"
              class="asset-ref-ctx-item ui-select-none"
              role="menuitem"
              :disabled="!messageContextCanFork"
              @click="doMessageFork"
            >
              <LucideIcon :icon="GitFork" :size="13" />
              {{ t("chat.messageMenu.forkFromMessage") }}
            </button>
          </template>
    </BaseContextMenu>

    <BaseContextMenu
      v-if="assetRefCtxMenu"
      class="asset-ref-ctx-menu"
      :x="assetRefCtxMenu.x"
      :y="assetRefCtxMenu.y"
      :min-width="184"
      @close="closeAssetRefContextMenu"
    >
          <button
            v-if="assetRefContextIsKnowledge"
            type="button"
            class="asset-ref-ctx-item"
            :disabled="!resourceActionsAvailable"
            @click="doAssetRefOpenInKnowledge"
          >
            <LucideIcon :icon="BookOpen" :size="13" />
            {{ t("common.openInKnowledge") }}
          </button>
          <button
            v-else-if="assetRefContextCanOpenInEditor"
            type="button"
            class="asset-ref-ctx-item"
            @click="doAssetRefOpenInEditor"
          >
            <LucideIcon :icon="ExternalLink" :size="13" />
            {{ t("common.openInEditor") }}
          </button>
          <button
            type="button"
            class="asset-ref-ctx-item"
            :disabled="!resourceActionsAvailable"
            @click="doAssetRefShowInFolder"
          >
            <LucideIcon :icon="FolderOpen" :size="13" />
            {{ t("common.openInFileExplorer") }}
          </button>
          <button type="button" class="asset-ref-ctx-item" @click="doAssetRefCopyPath">
            <LucideIcon :icon="Copy" :size="13" />
            {{ t("common.copyPath") }}
          </button>
          <template v-if="assetRefContextSupportsUnity">
            <div class="asset-ref-ctx-sep"></div>
            <button
              type="button"
              class="asset-ref-ctx-item"
              :class="{ disabled: !assetRefContextCanSelectInUnity }"
              :disabled="!assetRefContextCanSelectInUnity"
              @click="doAssetRefSelectInUnity"
            >
              <LucideIcon :icon="Crosshair" :size="13" />
              {{ t("common.selectInUnity") }}
            </button>
            <template v-if="assetRefContextCanOpenLocusInspector">
              <div class="asset-ref-ctx-sep"></div>
              <button
                type="button"
                class="asset-ref-ctx-item"
                @click="doAssetRefOpenInLocusInspector"
              >
                <LucideIcon :icon="PanelTopOpen" :size="13" />
                {{ t("common.openInLocusInspector") }}
              </button>
            </template>
          </template>
    </BaseContextMenu>

    <Transition name="lightbox">
      <div v-if="lightboxSrc" class="lightbox-overlay" @click="closeLightbox">
        <img :src="lightboxSrc" class="lightbox-img" @click.stop />
      </div>
    </Transition>
  </div><!-- /chat-view-layout -->
</template>

<style scoped>
.chat-view-layout {
  flex: 1 1 0;
  display: flex;
  width: 100%;
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.chat-view-layout.dragging-session {
  cursor: col-resize;
}

.chat-view-layout.is-vertical-layout {
  flex-direction: column;
}

.chat-view-layout.is-vertical-layout.dragging-session {
  cursor: default;
}

:deep(.session-panel) {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  background: var(--sidebar-bg);
  flex-shrink: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
  contain: layout paint;
}

.session-divider {
  width: 3px;
  flex-shrink: 0;
  cursor: col-resize;
  background: var(--border-color);
  transition: background 0.15s;
}

.session-divider:hover,
.chat-view-layout.dragging-session .session-divider {
  background: var(--text-secondary);
}

:deep(.sp-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px 4px;
}

:deep(.sp-title) {
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
}

:deep(.sp-header-actions) {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

:deep(.sp-collapse-btn) {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.15s ease, color 0.15s ease;
  box-shadow: none;
  padding: 0;
}

:deep(.sp-collapse-btn:hover),
:deep(.sp-collapse-btn:focus-visible) {
  background: var(--hover-bg);
  color: var(--text-color);
}

:deep(.sp-collapse-btn:focus-visible) {
  outline: none;
}

:deep(.sp-new-session-item) {
  width: 100%;
  font-family: inherit;
  text-align: left;
  color: var(--text-secondary);
}

:deep(.sp-new-session-item.active) {
  color: var(--text-color);
}

:deep(.sp-new-session-plus) {
  width: 12px;
  height: 12px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: currentColor;
  font-size: 13px;
  line-height: 1;
  opacity: 0.72;
}

:deep(.sp-session-list) {
  flex: 1 1 40%;
  min-height: 0;
  height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 2px 6px 10px;
}

:deep(.sp-session-item) {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 0;
  padding: 4px 6px;
  border-radius: 4px;
  border: 1px solid transparent;
  background: transparent;
  cursor: pointer;
  transition: background 0.12s;
  position: relative;
  overflow: hidden;
}

@supports (content-visibility: auto) {
  :deep(.sp-session-item) {
    content-visibility: auto;
    contain-intrinsic-size: auto 36px;
  }
}

:deep(.sp-session-item + .sp-session-item) {
  margin-top: 2px;
}

:deep(.sp-session-item:hover) {
  background: var(--hover-bg);
}

:deep(.sp-session-item.active) {
  background: color-mix(in srgb, var(--active-bg) 78%, var(--sidebar-bg));
  border-color: color-mix(in srgb, var(--accent-color) 18%, transparent);
}

:deep(.sp-session-item.role-dev) {}

:deep(.sp-session-item.role-subagent) {}

:deep(.sp-session-item.role-docgen) {}

:deep(.sp-session-item.role-knowledge) {}

:deep(.sp-session-item.role-git) {}

:deep(.sp-session-item.active) {
  border-color: color-mix(in srgb, var(--accent-color) 18%, transparent);
}

:deep(.sp-streaming-dot) {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #4caf50;
  margin-right: 4px;
  vertical-align: middle;
  animation: streaming-pulse 1.2s ease-in-out infinite;
}
@keyframes streaming-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

:deep(.sp-session-info) {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
}

:deep(.sp-session-main) {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  width: 100%;
}

:deep(.sp-session-title) {
  font-size: 13px;
  font-weight: 500;
  color: color-mix(in srgb, var(--text-secondary) 82%, var(--text-color) 18%);
  min-width: 0;
  flex: 1 1 auto;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.35;
  transition: color 0.12s ease;
}

:deep(.sp-session-item:hover .sp-session-title:not(.is-running)),
:deep(.sp-session-item.active .sp-session-title:not(.is-running)) {
  color: var(--text-color);
}

:deep(.sp-session-time) {
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  flex-shrink: 0;
  margin-left: auto;
  padding-left: 8px;
  opacity: 0.68;
  transition: opacity 0.12s ease;
}

:deep(.sp-delete-btn) {
  opacity: 0;
  width: 16px;
  height: 16px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  border-radius: 3px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  padding: 0;
  box-shadow: none;
  margin-left: auto;
  transition: opacity 0.12s ease, background 0.12s ease, color 0.12s ease;
}

:deep(.sp-session-item:hover .sp-delete-btn),
:deep(.sp-session-item.active .sp-delete-btn) {
  opacity: 1;
}

:deep(.sp-delete-btn:hover) {
  background: var(--hover-bg);
  color: #e55;
}

:deep(.sp-empty-hint) {
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
  padding: 24px 0;
}

.chat-view {
  --chat-workspace-content-max-width: 980px;
  z-index: 2;
  flex: 1 1 0;
  display: flex;
  flex-direction: column;
  width: 0;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: visible;
  position: relative;
  background: var(--msg-assistant-bg);
  contain: layout;
}

.chat-view.is-vertical-layout {
  width: 100%;
  flex-basis: auto;
}

.chat-main {
  position: relative;
  flex: 1 1 0;
  width: 100%;
  min-height: 0;
  min-width: 0;
  display: flex;
}

.chat-empty-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  z-index: 1;
  pointer-events: none;
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-secondary);
}

.empty-icon {
  width: 56px;
  height: 56px;
  border-radius: 16px;
  background: var(--active-bg);
  color: var(--text-color);
  font-size: 28px;
  font-weight: 800;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 8px;
}

.empty-title {
  font-size: 22px;
  font-weight: 700;
  color: var(--text-color);
}

.empty-subtitle {
  font-size: 14px;
}

.input-area {
  position: relative;
  flex: 0 0 auto;
  width: 100%;
  min-width: 0;
  padding: 12px 24px 18px;
  border-top: 0;
  background: transparent;
}

.input-area.is-controls-collapsed {
  padding-bottom: 14px;
}

.input-area.is-reference-drop-available {
  z-index: 41;
}

.chat-input-frame {
  position: relative;
  width: min(100%, var(--chat-workspace-content-max-width));
  min-width: 0;
  margin: 0 auto;
}

.chat-input-frame :deep(.chat-composer:not(.is-compact):not(.has-top-extension)) {
  min-height: 104px;
}

.input-backdrop-row {
  position: relative;
  z-index: 3;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  align-items: center;
  min-height: 24px;
  margin: 0 4px 6px;
}

.input-area.is-controls-collapsed .input-backdrop-row {
  min-height: 20px;
  margin-bottom: 4px;
}

.input-controls-toggle-zone {
  position: absolute;
  top: 10px;
  left: 0;
  z-index: 2;
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.input-area.is-controls-collapsed .input-controls-toggle-zone {
  top: 20px;
  left: 0;
}

.input-controls-toggle {
  width: 20px;
  height: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.2s ease, color 0.12s ease, background 0.12s ease;
}

.input-controls-toggle-zone:hover .input-controls-toggle,
.input-area.is-controls-switching .input-controls-toggle,
.input-controls-toggle:focus-visible {
  opacity: 1;
  pointer-events: auto;
}

.input-controls-toggle:hover,
.input-controls-toggle.is-collapsed:hover {
  color: var(--text-color);
  background: var(--hover-bg);
}

.input-controls-toggle.is-collapsed {
  color: var(--accent-color);
}

.input-controls-toggle-icon {
  width: 14px;
  height: 14px;
  display: block;
}

.input-backdrop-status {
  grid-column: 1;
  justify-self: start;
  display: flex;
  align-items: center;
}

.input-backdrop-action {
  grid-column: 2;
  justify-self: end;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  min-width: 0;
}

.chat-view.is-vertical-layout .input-area {
  padding: 10px 12px 12px;
}

.chat-view.is-vertical-layout .input-backdrop-row {
  margin-inline: 2px;
}

.chat-view.is-vertical-layout .input-controls-toggle-zone {
  top: 8px;
  left: 0;
}

.chat-view.is-vertical-layout .input-area.is-controls-collapsed .input-controls-toggle-zone {
  top: 18px;
}

.chat-pending-stack {
  width: min(100%, var(--chat-workspace-content-max-width));
  min-width: 0;
  margin-inline: auto;
}

.chat-view.is-vertical-layout :deep(.chat-transcript-scroll.is-session) {
  padding: 14px 0 0;
}

.chat-view.is-vertical-layout :deep(.chat-transcript-message.is-session) {
  padding-left: 16px;
  padding-right: 16px;
}

.chat-view.is-vertical-layout :deep(.chat-transcript-footer.is-session) {
  padding: 8px 16px 10px;
}

.chat-view.is-vertical-layout :deep(.chat-composer-footer) {
  align-items: flex-end;
  flex-wrap: wrap;
}

.chat-view.is-vertical-layout :deep(.chat-composer-footer-start) {
  flex: 1 1 auto;
}

.chat-view.is-vertical-layout :deep(.chat-composer-footer-end) {
  flex: 0 1 auto;
  align-self: flex-end;
  justify-content: flex-end;
  margin-left: auto;
  flex-wrap: nowrap;
}

.chat-view.is-vertical-layout :deep(.ask-user-card),
.chat-view.is-vertical-layout :deep(.knowledge-confirm-card),
.chat-view.is-vertical-layout :deep(.tool-confirm-batch-card) {
  margin-left: 12px;
  margin-right: 12px;
}

.queued-follow-up-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 6px 10px;
  margin: 0 12px 6px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg) 24%);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  font-size: 12px;
  color: var(--text-secondary);
}

.queued-follow-up-label {
  flex: 0 0 auto;
  color: var(--text-secondary);
}

.queued-follow-up-text {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  color: var(--text-color);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.queued-follow-up-insert,
.queued-follow-up-re-edit,
.queued-follow-up-delete {
  flex: 0 0 auto;
}

/* ── Mode selector ── */
/* Plan status bar */
.plan-status-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg) 24%);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  margin: 0 12px 6px;
  font-size: 12px;
  color: var(--text-secondary);
  box-shadow: none;
  animation: none;
}
.plan-status-icon {
  color: var(--accent-color);
  flex-shrink: 0;
}
.plan-status-exit {
  margin-left: auto;
  padding: 2px 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  cursor: pointer;
}
.plan-status-exit:hover:not(:disabled) {
  color: var(--text-primary);
  border-color: color-mix(in srgb, var(--accent-color) 40%, var(--border-color));
}
.plan-status-exit:disabled {
  opacity: 0.5;
  cursor: default;
}

.plan-confirm-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px;
  margin: 0 12px 6px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg) 24%);
  border: 1px solid color-mix(in srgb, var(--accent-color) 20%, var(--border-color));
  border-radius: 8px;
  font-size: 12px;
}
.plan-confirm-text {
  color: var(--text-secondary);
}
.plan-confirm-actions {
  display: flex;
  gap: 6px;
}
.plan-confirm-btn {
  min-height: 28px;
}

/* Icon button mirroring .chat-status-icon-btn so the row reads as one family. */
.changes-toggle-btn {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  min-width: 24px;
  height: 24px;
  min-height: 24px;
  line-height: 1;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  box-shadow: none;
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease, opacity 0.12s ease;
}

.changes-toggle-btn.is-active {
  background: var(--active-bg);
  border-color: color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  color: var(--text-color);
}

.changes-toggle-btn:hover:not(:disabled),
.changes-toggle-btn:focus-visible {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, currentColor 22%, transparent);
  color: var(--text-color);
}

.changes-toggle-btn:focus-visible {
  outline: none;
}

.changes-toggle-btn:disabled {
  opacity: 0.48;
  cursor: not-allowed;
}

/* Hover tooltip mirroring .chat-status-icon-label, anchored right at the panel edge. */
.changes-toggle-label {
  position: absolute;
  right: 0;
  bottom: calc(100% + 6px);
  z-index: 35;
  max-width: 180px;
  padding: 4px 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: var(--surface-elevated, var(--panel-bg));
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16);
  color: currentColor;
  pointer-events: none;
  overflow: hidden;
  font-size: 11px;
  line-height: 1.3;
  opacity: 0;
  transform: translateY(3px);
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: opacity 0.1s ease, transform 0.1s ease;
}

.changes-toggle-btn:not(.is-active):hover .changes-toggle-label,
.changes-toggle-btn:not(.is-active):focus-visible .changes-toggle-label {
  opacity: 1;
  transform: translateY(0);
}

/* ── Ask User Card ── */

:deep(.ask-user-card) {
  margin: 0 48px 12px;
  padding: 16px 20px;
  border: 1px solid var(--accent-color);
  border-radius: 12px;
  background: var(--msg-assistant-bg);
}

:deep(.knowledge-confirm-card) {
  margin: 0 48px 12px;
}

:deep(.tool-confirm-batch-card) {
  margin: 0 48px 12px;
}

:deep(.ask-question) {
  font-size: 14px;
  font-weight: 600;
  line-height: 1.5;
  margin-bottom: 12px;
  color: var(--text-color);
  white-space: pre-wrap;
}

:deep(.ask-options) {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

:deep(.ask-option-btn) {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  text-align: left;
  min-height: 0;
  padding: 10px 14px;
}

:deep(.ask-option-label) {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

:deep(.ask-option-desc) {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.4;
}

:deep(.ask-custom) {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

:deep(.ask-custom-label) {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

:deep(.ask-custom-input-row) {
  display: flex;
  gap: 6px;
  align-items: center;
}

:deep(.ask-custom-input) {
  flex: 1;
  padding: 8px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--bg-color);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  transition: border-color 0.15s;
}

:deep(.ask-custom-input:focus) {
  border-color: var(--accent-color);
}

:deep(.ask-custom-input::placeholder) {
  color: var(--text-secondary);
}

:deep(.ask-custom-send) {
  width: 32px;
  height: 32px;
  font-size: 16px;
  font-weight: 600;
  flex-shrink: 0;
  padding: 0;
}

:deep(.tool-confirm-card) {
  border-color: var(--warning-color, #e5a100);
}

:deep(.tool-confirm-header) {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}

:deep(.tool-confirm-icon) {
  color: var(--warning-color, #e5a100);
  display: flex;
  align-items: center;
}

:deep(.tool-confirm-title) {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

:deep(.tool-confirm-body) {
  margin-bottom: 12px;
}

:deep(.tool-confirm-name) {
  font-size: 13px;
  font-weight: 700;
  color: var(--accent-color);
  margin-bottom: 6px;
  font-family: var(--font-mono-identifier);
}

:deep(.tool-confirm-args) {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  padding: 8px 12px;
  margin: 0;
  max-height: 200px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: var(--font-mono-block);
}

:deep(.tool-confirm-actions) {
  display: flex;
  gap: 8px;
}

:deep(.tool-confirm-btn) {
  font-size: 13px;
  font-weight: 600;
  min-height: 32px;
}

.undo-chooser-fade-enter-active,
.undo-chooser-fade-leave-active {
  transition: opacity 120ms ease;
}

.undo-chooser-fade-enter-from,
.undo-chooser-fade-leave-to {
  opacity: 0;
}

.undo-chooser-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1300;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgba(0, 0, 0, 0.28);
}

.undo-chooser {
  width: min(360px, calc(100vw - 32px));
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--elevated-bg, var(--sidebar-bg));
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.22);
}

.undo-chooser-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 38px;
  padding: 0 8px 0 12px;
  border-bottom: 1px solid var(--border-color);
}

.undo-chooser-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.undo-chooser-close {
  width: 28px;
  height: 28px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
}

.undo-chooser-close:hover:not(:disabled),
.undo-chooser-close:focus-visible:not(:disabled) {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
}

.undo-chooser-close:disabled {
  cursor: default;
  opacity: 0.6;
}

.undo-chooser-actions {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px 12px;
}

.undo-chooser-action {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 34px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  font-weight: 500;
  text-align: left;
  cursor: pointer;
  box-shadow: none;
}

.undo-chooser-action:hover:not(:disabled),
.undo-chooser-action:focus-visible:not(:disabled) {
  border-color: var(--border-strong);
  background: var(--hover-bg);
  color: var(--text-color);
}

.undo-chooser-action.is-selected:not(:disabled) {
  border-color: var(--border-strong);
  background: var(--active-bg);
  color: var(--text-color);
}

.undo-chooser-action:disabled {
  cursor: default;
  opacity: 0.58;
}

.undo-chooser-action-state,
.undo-chooser-note,
.undo-chooser-empty {
  font-size: 12px;
  color: var(--text-secondary);
}

.undo-chooser-note {
  padding: 0 2px 2px;
}

.undo-chooser-warning {
  padding: 8px 10px;
  border: 1px solid var(--warning-border, var(--border-color));
  border-radius: 6px;
  background: var(--warning-bg, var(--bg-color));
  color: var(--warning-text, var(--text-color));
  font-size: 12px;
  line-height: 1.5;
}

.undo-chooser-warning-files {
  margin-top: 4px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: pre-line;
  word-break: break-word;
  max-height: 120px;
  overflow-y: auto;
}

.undo-chooser-empty {
  padding: 14px;
}

.lightbox-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.7);
  backdrop-filter: blur(4px);
  cursor: zoom-out;
}

.lightbox-img {
  max-width: 90vw;
  max-height: 90vh;
  border-radius: 8px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  cursor: default;
  object-fit: contain;
}

.lightbox-enter-active {
  transition: opacity 0.2s ease;
}
.lightbox-leave-active {
  transition: opacity 0.15s ease;
}
.lightbox-enter-from,
.lightbox-leave-to {
  opacity: 0;
}

/* ── Inline diff panel (matches CollabView layout) ── */
.diff-inline-panel {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
.diff-inline-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}
.diff-inline-path {
  font-family: var(--font-mono-identifier);
  font-size: 13px;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.diff-inline-stats {
  font-size: 12px;
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
.diff-inline-stats .stat-add { color: #38a169; }
.diff-inline-stats .stat-del { color: #e53e3e; }
.diff-inline-status {
  font-size: 11px;
  font-weight: 700;
  padding: 1px 6px;
  border-radius: 3px;
  flex-shrink: 0;
}
.diff-inline-status.status-m { background: #d29b0022; color: #d29b00; }
.diff-inline-status.status-a { background: #2ea04322; color: #2ea043; }
.diff-inline-status.status-d { background: #e1575922; color: #e15759; }
.diff-inline-status.status-r { background: #388bfd22; color: #388bfd; }
.diff-inline-actions {
  display: flex;
  gap: 6px;
  margin-left: auto;
  flex-shrink: 0;
}
.diff-inline-tab-group {
  align-self: center;
}
.diff-inline-action-btn {
  gap: 4px;
  font-size: 11px;
  white-space: nowrap;
  min-height: 26px;
  padding: 0 8px;
}
.diff-close-btn {
  width: 28px;
  height: 28px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 18px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  flex-shrink: 0;
}
.diff-close-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}
.diff-inline-loading,
.diff-inline-error {
  padding: 24px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
}
.diff-progress-info {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  min-width: 200px;
}
.diff-progress-text {
  font-size: 13px;
  color: var(--text-secondary);
}
.diff-progress-bar {
  width: 100%;
  height: 3px;
  background: var(--border-color, rgba(255, 255, 255, 0.08));
  border-radius: 2px;
  overflow: hidden;
}
.diff-progress-fill {
  height: 100%;
  background: var(--accent-color, #58a6ff);
  border-radius: 2px;
  transition: width 0.3s ease;
}
.diff-inline-error {
  color: #e53e3e;
}
.diff-inline-body {
  flex: 1;
  overflow: auto;
  min-height: 0;
}
</style>
