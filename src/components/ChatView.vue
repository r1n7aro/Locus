
<script setup lang="ts">
import { ref, nextTick, watch, computed, onMounted, onUnmounted } from "vue";
import { PanelTopOpen } from "lucide";
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
import type { ChatComposerSendPayload, ChatMessage, AgentInfo, TokenUsage, ModelOption, PendingQuestion, PendingToolConfirm, EffortLevel, SessionSummary, AssetDbScanEvent, ScanStats, ImageAttachment, AssetRefAttachment, SkillManifest, UserIntentMeta, SaveRawContextRequest, CodexTransportMode, AssistantRenderPart, UnityConnectionStatus, KnowledgeDocumentType } from "../types";
import type { ToolCallDisplay } from "../types";
import ModelEffortSelector from "./ModelEffortSelector.vue";
import SessionPanel from "./chat/SessionPanel.vue";
import SessionCompactPicker from "./chat/SessionCompactPicker.vue";
import ChatTranscript from "./chat/ChatTranscript.vue";
import ChatStatusIndicators from "./chat/ChatStatusIndicators.vue";
import RichChatInput from "./chat/RichChatInput.vue";
import TokenUsageBar from "./chat/TokenUsageBar.vue";
import AskUserCard from "./chat/AskUserCard.vue";
import ToolConfirmCard from "./chat/ToolConfirmCard.vue";
import ToolConfirmBatchCard from "./chat/ToolConfirmBatchCard.vue";
import FileDiffViewer from "./diff/FileDiffViewer.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { clampFloatingPosition } from "./ui/floatingPosition";
import { refetchDiffByKey } from "../services/diff";
import { openChatDiffReviewWindow } from "../services/chatDiffReviewWindow";
import { normalizeAppError } from "../services/errors";
import { knowledgeRevealTarget } from "../services/knowledge";
import { t } from "../i18n";
import { useChatChangesStore } from "../stores/chatChanges";
import { useChatStore } from "../stores/chat";
import { useUiStore } from "../stores/ui";
import { useNotificationStore } from "../stores/notification";
import {
  captureScrollAnchor,
  captureLiveScrollAnchor,
  captureSessionScrollState,
  resolveSessionScrollTop,
  restoreLiveScrollAnchor,
  restoreScrollAnchor,
  type LiveScrollAnchorSnapshot,
} from "../composables/chatScrollState";
import {
  createCoalescedScrollScheduler,
  createSettledScrollScheduler,
  hasRunningToolCall,
  shouldAutoScrollToBottom,
  shouldShowWaitingPlaceholder,
} from "../composables/chatViewStability";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../composables/resizeObserver";
import { forwardWheelToElement } from "../composables/chatWheelPassthrough";
import { canOpenInEditor } from "../composables/useHideMeta";
import { useDiffProgress } from "../composables/useDiffProgress";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import { matchesShortcut, useKeyboardShortcuts } from "../composables/useKeyboardShortcuts";
import { getLocusRuntime } from "../services/locusRuntime";
import {
  getChatSubmitModifierLabel,
  useChatInputSettings,
} from "../composables/useChatInputSettings";
import { logToolCollapseTrace, previewTraceText } from "../services/toolCollapseTrace";
import { recordLayoutDiagnostic } from "../services/layoutDiagnostics";

type ChatLayoutMode = "auto" | "horizontal" | "vertical";
type ResolvedChatLayoutMode = "horizontal" | "vertical";

const chatChangesStore = useChatChangesStore();
const chatStore = useChatStore();
const uiStore = useUiStore();
const notificationStore = useNotificationStore();
const { state: shortcutState } = useKeyboardShortcuts();
const { state: chatInputSettings } = useChatInputSettings();

const isPlanStreaming = computed(() => !!chatStore.pendingPlanRun && props.isStreaming);
const isPlanDone = computed(() => !!chatStore.pendingPlanRun && !props.isStreaming);

const isViewingSubagent = computed(() => {
  if (!props.activeSessionId) return false;
  const session = props.sessions.find(s => s.id === props.activeSessionId);
  return !!session?.parentSessionId;
});
const diffProgress = useDiffProgress();
const diffProgressWidth = computed(() => `${diffProgress.progress.value * 100}%`);
const chatInputPlaceholder = computed(() => {
  if (chatInputSettings.submitMode === "mod-enter-send") {
    return t("chat.input.placeholderModifierSend", getChatSubmitModifierLabel());
  }
  return t("chat.input.placeholder");
});
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

const showInlineDiff = computed(() =>
  !!chatChangesStore.inlineDiffPayload || chatChangesStore.inlineDiffLoading || !!chatChangesStore.inlineDiffError,
);
const hasPanelToggleRow = computed(() => chatChangesStore.currentFileCount > 0);

const chatDiffViewerRef = ref<InstanceType<typeof FileDiffViewer> | null>(null);
const chatDiffTabOptions = computed(() => [
  { value: "semantic", label: t("diff.tabs.semantic") },
  { value: "text", label: t("diff.tabs.text") },
]);

watch(() => chatChangesStore.inlineDiffLoading, (loading) => {
  if (loading) diffProgress.reset();
});

async function onChatDiffLfsPulled() {
  const payload = chatChangesStore.inlineDiffPayload;
  if (!payload) return;
  try {
    const updated = await refetchDiffByKey(payload.key);
    if (updated) chatChangesStore.inlineDiffPayload = updated;
  } catch (e) {
    console.error("[ChatView] refetch after LFS pull failed:", e);
  }
}

const props = defineProps<{
  messages: ChatMessage[];
  streamingText: string;
  streamingTextOrder?: number;
  isStreaming: boolean;
  isCompacting: boolean;
  isThinking: boolean;
  hasThinking: boolean;
  thinkingText: string;
  thinkingOrder?: number;
  thinkingDuration: number;
  liveRenderParts?: AssistantRenderPart[];
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
  tokenUsage: TokenUsage;
  pendingQuestion: PendingQuestion | null;
  pendingToolConfirms: PendingToolConfirm[];
  sessions: SessionSummary[];
  activeSessionId: string | null;
  unityConnected?: boolean;
  unityPluginStatus?: "missing" | "outdated" | null;
  unityPluginInstalling?: boolean;
  unityLaunching?: boolean;
  unityLaunchState?: "idle" | "starting" | "waitingConnection";
  unityConnectionStatus?: UnityConnectionStatus | null;
  workingDir?: string;
  scanPhase?: AssetDbScanEvent | null;
  lastScanStats?: ScanStats | null;
  isUnityProject?: boolean;
  skills?: SkillManifest[];
  streamingSessionIds?: Set<string>;
  undoableMessageIds?: Set<string>;
  layoutMode?: ChatLayoutMode;
  defaultSessionPanelCollapsed?: boolean;
  sessionPanelStorageScope?: string;
}>();

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
  cancel: [];
  selectAgent: [id: string];
  selectModel: [id: string];
  selectEffort: [level: EffortLevel];
  saveRawContext: [request: SaveRawContextRequest];
  answerQuestion: [answer: string];
  answerToolConfirm: [questionId: string, answer: string];
  answerAllToolConfirms: [questionIds: string[], answer: string];
  openThinking: [content: string];
  selectSession: [id: string];
  newChat: [];
  renameSession: [id: string, title: string];
  archiveSession: [id: string];
  deleteSession: [id: string];
  startScan: [];
  installPlugin: [];
  launchUnityProject: [];
  layoutModeChange: [mode: ResolvedChatLayoutMode];
}>();

const lightboxSrc = ref("");
function openLightbox(src: string) {
  lightboxSrc.value = src;
}
function closeLightbox() {
  lightboxSrc.value = "";
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

type AssetRefContextMenuState = {
  x: number;
  y: number;
  target: AssetRefContextMenuTarget;
};

const ASSET_REF_CONTEXT_MENU_VIEWPORT_MARGIN = 8;
const KNOWLEDGE_DOCUMENT_ROOT_RE = /^(design|memory|skill|reference)\/(.+\.md)$/i;
const KNOWLEDGE_DOCUMENT_FILE_RE = /^Locus\/knowledge\/(design|memory|skill|reference)\/(.+\.md)$/i;
const assetRefCtxMenu = ref<AssetRefContextMenuState | null>(null);
const assetRefCtxMenuRef = ref<HTMLElement | null>(null);
const assetRefCtxMenuStyle = computed(() => ({
  left: `${assetRefCtxMenu.value?.x ?? 0}px`,
  top: `${assetRefCtxMenu.value?.y ?? 0}px`,
}));

function isUnityRuntime() {
  return getLocusRuntime().kind === "unity";
}

function isUnityEmbeddedWindow() {
  if (isUnityRuntime()) return true;
  if (typeof window === "undefined") return false;
  return window.location.pathname === "/unity-embed";
}

function isUnityAssetPath(filePath: string) {
  return /^(Assets|Packages)\//.test(filePath.replace(/\\/g, "/"));
}

function shouldSelectUnityAsset(filePath: string) {
  return isUnityAssetPath(filePath) && (props.unityConnected || isUnityRuntime());
}

function shouldOpenUnityAssetInspector(e: MouseEvent, filePath: string) {
  return (e.ctrlKey || e.metaKey)
    && isUnityEmbeddedWindow()
    && isUnityAssetPath(filePath)
    && !canOpenInEditor(filePath);
}

function shouldUseUnitySceneObjectRef(scenePath: string, objectPath: string) {
  return /\.unity$/i.test(scenePath.replace(/\\/g, "/"))
    && objectPath.trim().length > 0
    && (props.unityConnected || isUnityRuntime());
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

const assetRefContextIsKnowledge = computed(() =>
  assetRefCtxMenu.value?.target.kind === "knowledge",
);

const assetRefContextCanOpenInEditor = computed(() => {
  const target = assetRefCtxMenu.value?.target;
  return !!target && !(target.kind === "knowledge" || (target.kind === "file" && target.entryKind === "folder"));
});

const assetRefContextSupportsUnity = computed(() => {
  const target = assetRefCtxMenu.value?.target;
  return target?.kind === "asset" || target?.kind === "sceneObject";
});

function viewportSize() {
  return {
    width: window.innerWidth || document.documentElement.clientWidth,
    height: window.innerHeight || document.documentElement.clientHeight,
  };
}

function updateAssetRefContextMenuPosition() {
  const menu = assetRefCtxMenu.value;
  const el = assetRefCtxMenuRef.value;
  if (!menu || !el) return;

  const rect = el.getBoundingClientRect();
  const next = clampFloatingPosition(
    { x: menu.x, y: menu.y },
    { width: rect.width, height: rect.height },
    viewportSize(),
    ASSET_REF_CONTEXT_MENU_VIEWPORT_MARGIN,
  );
  if (next.x === menu.x && next.y === menu.y) return;
  assetRefCtxMenu.value = { ...menu, x: next.x, y: next.y };
}

function closeAssetRefContextMenu() {
  assetRefCtxMenu.value = null;
}

function normalizeAssetRefDatasetPath(path: string | undefined): string {
  return (path ?? "").trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function toKnowledgeDocumentType(value: string): KnowledgeDocumentType | null {
  const normalized = value.trim().toLowerCase();
  if (
    normalized === "design" ||
    normalized === "memory" ||
    normalized === "skill" ||
    normalized === "reference"
  ) {
    return normalized;
  }
  return null;
}

function parseKnowledgeDocumentRefPath(filePath: string): AssetRefContextMenuTarget | null {
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
  const knowledgeRef = target.closest(".asset-chip[data-ref-kind='knowledge']") as HTMLElement | null;
  if (knowledgeRef) {
    const docType = toKnowledgeDocumentType(knowledgeRef.dataset.knowledgeType ?? "");
    const path = normalizeAssetRefDatasetPath(knowledgeRef.dataset.knowledgePath);
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
        entryKind: workspaceAssetRef?.dataset.entryKind === "folder" ? "folder" : "file",
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
  const target = assetContextTargetFromElement(e.target);
  if (!target) return;

  e.preventDefault();
  e.stopPropagation();
  assetRefCtxMenu.value = {
    x: e.clientX,
    y: e.clientY,
    target,
  };
  void nextTick(updateAssetRefContextMenuPosition);
}

function handleContentClick(e: MouseEvent) {
  const target = e.target as HTMLElement;
  if (target.tagName === "IMG") {
    e.preventDefault();
    openLightbox((target as HTMLImageElement).src);
    return;
  }
  const workspaceRef = target.closest(".md-workspace-ref") as HTMLElement | null;
  if (workspaceRef) {
    e.preventDefault();
    const workspacePath = workspaceRef.dataset.workspacePath;
    const entryKind = workspaceRef.dataset.entryKind;
    if (!workspacePath) return;
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
    const assetPath = fileRef.dataset.assetPath || filePath;
    if (shouldOpenUnityAssetInspector(e, assetPath)) {
      handleUnityAssetInspectorClick(assetPath);
      return;
    }
    handleFileRefClick(filePath);
  }
}

function handleUnityAssetInspectorClick(filePath: string) {
  openUnityAssetInspector(filePath).catch((e: unknown) => {
    console.warn("openUnityAssetInspector failed:", e);
    handleFileRefClick(filePath);
  });
}

function handleUnitySceneObjectInspectorClick(scenePath: string, objectPath: string) {
  openUnitySceneObjectInspector(scenePath, objectPath).catch((e: unknown) => {
    console.warn("openUnitySceneObjectInspector failed:", e);
    notifyUnitySceneObjectError(e, scenePath, objectPath);
  });
}

function handleUnitySceneObjectClick(scenePath: string, objectPath: string) {
  if (!shouldUseUnitySceneObjectRef(scenePath, objectPath)) return;
  selectUnitySceneObject(scenePath, objectPath).catch((e: unknown) => {
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
  if (canOpenInEditor(filePath)) {
    openFileExternal(filePath).catch((e: unknown) => console.warn("openFileExternal failed:", e));
    return;
  }
  if (shouldSelectUnityAsset(filePath)) {
    selectUnityAsset(filePath).catch((e: unknown) => console.warn("selectUnityAsset failed:", e));
    return;
  }
  openFileExternal(filePath).catch((e: unknown) => console.warn("openFileExternal failed:", e));
}

function handleFolderRefClick(folderPath: string) {
  if (shouldSelectUnityAsset(folderPath)) {
    selectUnityAsset(folderPath).catch((e: unknown) => console.warn("selectUnityAsset failed:", e));
    return;
  }
  showInFolder(folderPath).catch((e: unknown) => console.warn("showInFolder failed:", e));
}

function notifyAssetRefContextMenuError(error: unknown, operation: string, fallbackMessage: string) {
  const err = normalizeAppError(error);
  notificationStore.addNotice("warning", err.message || fallbackMessage, {
    code: err.code,
    operation,
    replaceOperation: true,
  });
}

async function doAssetRefOpenInEditor() {
  const target = assetRefCtxMenu.value?.target;
  if (!target || !assetRefContextCanOpenInEditor.value) return;
  closeAssetRefContextMenu();
  try {
    await openFileExternal(target.filePath);
  } catch (error) {
    console.warn("openFileExternal failed:", error);
    notifyAssetRefContextMenuError(error, "assetRefOpenInEditor", "Failed to open file");
  }
}

async function openInlineDiffInWindow() {
  const payload = chatChangesStore.inlineDiffPayload;
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

async function doAssetRefShowInFolder() {
  const target = assetRefCtxMenu.value?.target;
  if (!target) return;
  closeAssetRefContextMenu();
  try {
    if (target.kind === "knowledge") {
      await knowledgeRevealTarget({
        kind: "document",
        docType: target.docType,
        path: target.path,
      });
      return;
    }
    await showInFolder(target.filePath);
  } catch (error) {
    console.warn("showInFolder failed:", error);
    notifyAssetRefContextMenuError(error, "assetRefShowInFolder", "Failed to show file in folder");
  }
}

function doAssetRefOpenInKnowledge() {
  const target = assetRefCtxMenu.value?.target;
  if (!target || target.kind !== "knowledge") return;
  closeAssetRefContextMenu();
  uiStore.stageKnowledgeSelection({
    dashboard: target.docType,
    path: target.path,
  });
  uiStore.setTab("knowledge");
}

async function doAssetRefSelectInUnity() {
  const target = assetRefCtxMenu.value?.target;
  if (!target || !assetRefContextCanSelectInUnity.value) return;
  closeAssetRefContextMenu();

  try {
    if (target.kind === "sceneObject") {
      await selectUnitySceneObject(target.scenePath, target.objectPath);
      return;
    }
    if (target.kind !== "asset") return;
    await selectUnityAsset(target.assetPath);
  } catch (error) {
    console.warn("selectUnityAsset failed:", error);
    if (target.kind === "sceneObject") {
      notifyUnitySceneObjectError(error, target.scenePath, target.objectPath);
      return;
    }
    notifyAssetRefContextMenuError(error, "assetRefSelectInUnity", "Failed to select asset in Unity");
  }
}

function handleQuestionAnswer(answer: string) {
  emit("answerQuestion", answer);
}

const NEW_CHAT_DRAFT_KEY = "__new_chat__";
const inputText = ref("");
const composerDrafts = ref(new Map<string, string>());
const composerPanelRef = ref<InstanceType<typeof RichChatInput> | null>(null);
const transcriptRef = ref<InstanceType<typeof ChatTranscript> | null>(null);

function draftSessionKey(sessionId: string | null) {
  return sessionId ?? NEW_CHAT_DRAFT_KEY;
}

const composerAssetRefSyncKey = computed(() => `chat:${draftSessionKey(props.activeSessionId)}`);

function storeComposerDraft(sessionId: string | null, value: string) {
  const key = draftSessionKey(sessionId);
  if (value) {
    composerDrafts.value.set(key, value);
    return;
  }
  composerDrafts.value.delete(key);
}

async function restoreComposerDraft(sessionId: string | null) {
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

async function handleNewChatRequest() {
  if (props.activeSessionId === null) {
    composerPanelRef.value?.resetDraft();
    inputText.value = "";
  }
  emit("newChat");
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

watch(
  () => uiStore.pendingChatPrefill?.id,
  async (prefillId) => {
    const prefill = uiStore.pendingChatPrefill;
    if (!prefillId || !prefill) return;
    await applyExternalComposerPrefill(prefill.text);
    uiStore.clearPendingChatPrefill(prefillId);
  },
);

watch(inputText, (value) => {
  storeComposerDraft(props.activeSessionId, value);
});

const hasStreamingContent = computed(
  () => !!displayedStreamingText.value || hasRunningToolCall(props.activeToolCalls)
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
let suppressScrollCapture = false;
let activeToolViewportAnchor: LiveScrollAnchorSnapshot | null = null;
let toolViewportAnchorFrame = 0;
const displayedStreamingText = ref("");
let pendingStreamingText = "";
let streamingTextFlushTimer: ReturnType<typeof setTimeout> | null = null;
const STREAMING_TEXT_RENDER_DELAY_MS = 80;
const STREAM_END_SCROLL_SETTLE_MS = 320;

function clearStreamingTextFlushTimer() {
  if (!streamingTextFlushTimer) return;
  clearTimeout(streamingTextFlushTimer);
  streamingTextFlushTimer = null;
}

function flushDisplayedStreamingText() {
  logToolCollapseTrace("chat-view", "flushDisplayedStreamingText", {
    pendingLen: pendingStreamingText.length,
    pendingPreview: previewTraceText(pendingStreamingText, 64),
    previousDisplayedLen: displayedStreamingText.value.length,
  });
  displayedStreamingText.value = pendingStreamingText;
  streamingTextFlushTimer = null;
}

watch(
  () => props.streamingText,
  (nextText, previousText = "") => {
    pendingStreamingText = nextText;
    logToolCollapseTrace("chat-view", "sourceStreamingTextChanged", {
      previousLen: previousText.length,
      nextLen: nextText.length,
      displayedLen: displayedStreamingText.value.length,
      hasFlushTimer: !!streamingTextFlushTimer,
      nextPreview: nextText ? previewTraceText(nextText, 64) : "",
    });
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

function handleBottomPanelWheel(event: WheelEvent) {
  forwardWheelToElement(event, getMessagesElement());
}

function captureCurrentSessionScrollState(el: HTMLElement): ReturnType<typeof captureSessionScrollState> {
  return captureSessionScrollState(readMessageMetrics(el), captureScrollAnchor(el));
}

function rememberScrollForSession(sessionId: string | null = props.activeSessionId) {
  const el = getMessagesElement();
  if (!sessionId || !el) return;
  chatStore.rememberSessionScrollState(sessionId, captureCurrentSessionScrollState(el));
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
    chatStore.rememberSessionScrollState(sessionId, captureCurrentSessionScrollState(el));
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
  if (!el.contains(anchorState.anchor)) {
    clearToolViewportAnchor();
    return false;
  }

  suppressScrollCapture = true;
  const restored = restoreLiveScrollAnchor(el, anchorState);
  if (restored && props.activeSessionId) {
    chatStore.rememberSessionScrollState(props.activeSessionId, captureCurrentSessionScrollState(el));
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

  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  streamEndScrollScheduler.cancel();
  clearToolViewportAnchorFrame();
  activeToolViewportAnchor = captureLiveScrollAnchor(el, anchor);
  restoreToolViewportAnchor();
}

function handleToolViewportAnchorEnd(anchor: HTMLElement) {
  if (!activeToolViewportAnchor || activeToolViewportAnchor.anchor !== anchor) return;

  restoreToolViewportAnchor();
  clearToolViewportAnchorFrame();
  toolViewportAnchorFrame = requestViewportFrame(() => {
    toolViewportAnchorFrame = 0;
    restoreToolViewportAnchor();
    activeToolViewportAnchor = null;
  });
}

function setMessagesScrollTop(scrollTop: number, sessionId: string | null = props.activeSessionId) {
  runProgrammaticScrollUpdate((el) => {
    el.scrollTop = scrollTop;
  }, sessionId);
}

function restoreMessagesScrollState(
  state: ReturnType<typeof chatStore.getSessionScrollState>,
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

function isPendingSessionRestoreAwaitingMessages() {
  const targetSessionId = pendingRestoreSessionId.value;
  return !!targetSessionId
    && targetSessionId === props.activeSessionId
    && pendingRestoreMessagesRef.value === props.messages;
}

function scrollToBottomNow(force = false) {
  if (isPendingSessionRestoreAwaitingMessages()) return;

  const el = getMessagesElement();
  if (!el) return;

  const metrics = readMessageMetrics(el);
  const remembered = props.activeSessionId ? chatStore.getSessionScrollState(props.activeSessionId) : null;
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

    const sessionId = props.activeSessionId;
    const remembered = sessionId ? chatStore.getSessionScrollState(sessionId) : null;
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

const streamEndScrollScheduler = createSettledScrollScheduler(
  () => scrollToBottom(true),
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
    streamEndScrollScheduler.cancel();
    return;
  }
  if (previousQuiet) {
    reconcileViewport();
  }
});

function reconcileViewport(forceBottom = false) {
  if (toolHandoffViewportQuiet.value) return;
  if (restoreToolViewportAnchor()) return;
  if (pendingRestoreSessionId.value && pendingRestoreSessionId.value === props.activeSessionId) {
    restorePendingSessionScroll();
    return;
  }

  const el = getMessagesElement();
  if (!el) return;

  const remembered = props.activeSessionId ? chatStore.getSessionScrollState(props.activeSessionId) : null;
  if (shouldAutoScrollToBottom({ force: forceBottom, metrics: readMessageMetrics(el), remembered })) {
    scrollToBottom(forceBottom);
    return;
  }

  preserveScrollAnchor();
}

function settleStreamEndScroll() {
  if (toolHandoffViewportQuiet.value) return;
  const el = getMessagesElement();
  if (!el) return;

  const metrics = readMessageMetrics(el);
  const remembered = props.activeSessionId ? chatStore.getSessionScrollState(props.activeSessionId) : null;
  if (!shouldAutoScrollToBottom({ metrics, remembered })) {
    preserveScrollAnchor();
    return;
  }

  streamEndScrollScheduler.schedule();
}

function finishPendingSessionRestore(targetSessionId: string) {
  if (pendingRestoreSessionId.value !== targetSessionId || props.activeSessionId !== targetSessionId) return;
  pendingRestoreSessionId.value = null;
  pendingRestoreMessagesRef.value = null;
}

function restorePendingSessionScroll(options: { defer?: boolean } = {}) {
  const targetSessionId = pendingRestoreSessionId.value;
  if (!targetSessionId || targetSessionId !== props.activeSessionId) return;
  if (isPendingSessionRestoreAwaitingMessages()) return;

  const restore = () => {
    const el = getMessagesElement();
    if (!el || pendingRestoreSessionId.value !== props.activeSessionId) return;

    const remembered = chatStore.getSessionScrollState(targetSessionId);
    restoreMessagesScrollState(remembered, targetSessionId);
    finishPendingSessionRestore(targetSessionId);
  };

  if (options.defer) {
    nextTick(restore);
    return;
  }

  restore();
}

function onMessagesScroll() {
  if (suppressScrollCapture) return;
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  streamEndScrollScheduler.cancel();
  rememberScrollForSession();
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
  return uiStore.isWindowResizing || isDraggingSession.value;
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
  if (suppressScrollCapture || toolHandoffViewportQuiet.value) return;
  if (restoreToolViewportAnchor()) return;
  if (pendingRestoreSessionId.value && pendingRestoreSessionId.value === props.activeSessionId) {
    restorePendingSessionScroll();
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
    clearToolViewportAnchor();
    scrollToBottomScheduler.cancel();
    streamEndScrollScheduler.cancel();
    preserveScrollAnchorScheduler.cancel();
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
watch(() => props.messages.length, () => reconcileViewport());
watch(() => displayedStreamingText.value, () => reconcileViewport());
watch(() => props.activeToolCalls, () => reconcileViewport(), { deep: true });
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

function handlePlanContinue() {
  chatStore.clearPendingPlan();
  emit("send", t("chat.plan.continueMessage"), [], []);
}

function handleComposerSend(payload: ChatComposerSendPayload) {
  if (chatStore.pendingPlanRun) {
    chatStore.clearPendingPlan();
  }

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
  !showInlineDiff.value && !isVerticalLayout.value && !sessionPanelCollapsed.value,
);
const showSessionCompactPicker = computed(() => isVerticalLayout.value || sessionPanelCollapsed.value);

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

function onGlobalChatKeydown(e: KeyboardEvent) {
  if (uiStore.activeTab !== "chat") return;
  if (!e.repeat && matchesShortcut(e, shortcutState.newChat)) {
    e.preventDefault();
    handleNewChatRequest();
    return;
  }
  if (e.key === "Escape" && showInlineDiff.value) {
    chatChangesStore.closeInlineDiff();
  }
}

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
  rememberScrollForSession();
  clearInputControlsSwitchTimer();
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  streamEndScrollScheduler.cancel();
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
      <template v-if="chatChangesStore.inlineDiffPayload">
        <div class="diff-inline-header">
          <span class="diff-inline-status" :class="'status-' + (chatChangesStore.inlineDiffPayload.status ?? '').toLowerCase()">
            {{ chatChangesStore.inlineDiffPayload.status }}
          </span>
          <span v-if="chatChangesStore.inlineDiffPayload.oldPath" class="diff-inline-path" :title="chatChangesStore.inlineDiffPayload.oldPath + ' → ' + chatChangesStore.inlineDiffPayload.filePath">
            {{ chatChangesStore.inlineDiffPayload.oldPath }} → {{ chatChangesStore.inlineDiffPayload.filePath }}
          </span>
          <span v-else class="diff-inline-path" :title="chatChangesStore.inlineDiffPayload.filePath">
            {{ chatChangesStore.inlineDiffPayload.filePath }}
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
            <span class="stat-add">+{{ chatChangesStore.inlineDiffPayload.stats.additions }}</span>
            <span class="stat-del">-{{ chatChangesStore.inlineDiffPayload.stats.deletions }}</span>
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
              v-if="!chatChangesStore.inlineDiffPayload!.isBinary && canOpenInEditor(chatChangesStore.inlineDiffPayload!.filePath)"
              class="diff-inline-action-btn ui-select-none"
              :title="t('common.openInEditor')"
              @click="openFileExternal(chatChangesStore.inlineDiffPayload!.filePath)"
            >
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
              {{ t('common.openInEditor') }}
            </BaseButton>
          </span>
          <button class="diff-close-btn ui-select-none" @click="chatChangesStore.closeInlineDiff()">&times;</button>
        </div>
        <div class="diff-inline-body">
          <FileDiffViewer
            ref="chatDiffViewerRef"
            :payload="chatChangesStore.inlineDiffPayload"
            :hide-builtin-tabs="true"
            @lfs-pulled="onChatDiffLfsPulled"
          />
        </div>
      </template>
      <div v-else-if="chatChangesStore.inlineDiffLoading" class="diff-inline-loading">
        <div class="diff-progress-info">
          <span class="diff-progress-text">{{ diffProgress.phaseLabel }}</span>
          <div class="diff-progress-bar">
            <div class="diff-progress-fill" :style="{ width: diffProgressWidth }"></div>
          </div>
        </div>
      </div>
      <div v-else-if="chatChangesStore.inlineDiffError" class="diff-inline-error">{{ chatChangesStore.inlineDiffError }}</div>
    </div>

    <SessionPanel
      v-show="showSessionPanel"
      :sessions="sessions"
      :active-session-id="activeSessionId"
      :streaming-session-ids="streamingSessionIds"
      :session-panel-width="sessionPanelWidth"
      @select-session="emit('selectSession', $event)"
      @new-chat="handleNewChatRequest"
      @rename-session="(id: string, title: string) => emit('renameSession', id, title)"
      @archive-session="emit('archiveSession', $event)"
      @delete-session="emit('deleteSession', $event)"
      @save-raw-context="emit('saveRawContext', $event)"
      @toggle-panel-collapsed="setSessionPanelCollapsed(true)"
    />

    <div v-show="showSessionPanel" class="session-divider" @mousedown="onSessionSplitterMouseDown"></div>

    <div
      v-show="!showInlineDiff"
      class="chat-view"
      :class="{ 'is-vertical-layout': isVerticalLayout }"
    >
      <SessionCompactPicker
        v-if="showSessionCompactPicker"
        :sessions="sessions"
        :active-session-id="activeSessionId"
        :streaming-session-ids="streamingSessionIds"
        :show-expand-panel-button="sessionPanelCollapsed && !isVerticalLayout"
        @select-session="emit('selectSession', $event)"
        @new-chat="handleNewChatRequest"
        @expand-panel="setSessionPanelCollapsed(false)"
      />
      <div class="chat-main">
        <ChatTranscript
          ref="transcriptRef"
          variant="session"
          :session-key="activeSessionId || NEW_CHAT_DRAFT_KEY"
          :messages="messages"
          :streaming-text="displayedStreamingText"
          :streaming-text-order="streamingTextOrder"
          :is-streaming="isStreaming"
          :is-compacting="isCompacting"
          :is-thinking="isThinking"
          :has-thinking="hasThinking"
          :thinking-order="thinkingOrder"
          :thinking-duration="thinkingDuration"
          :live-render-parts="liveRenderParts"
          :active-tool-calls="activeToolCalls"
          user-label="You"
          assistant-label="Locus"
          :handoff-label="t('chat.transcript.handoff')"
          :waiting-label="t('chat.transcript.waiting')"
          :compacting-label="t('chat.transcript.compacting')"
          :compacted-label="t('chat.transcript.compacted')"
          :thinking-active-label="t('chat.transcript.thinking')"
          :thought-duration-label="t('chat.transcript.thoughtDuration', '{0}')"
          :thought-moment-label="t('chat.transcript.thoughtMoment')"
          enable-intent-badges
          show-user-images
          user-content-mode="asset"
          @scroll="onMessagesScroll"
          @content-click="handleContentClick"
          @content-contextmenu="handleContentContextMenu"
          @open-thinking="emit('openThinking', $event)"
          @open-image="openLightbox"
          @apply-knowledge-proposal="chatStore.applyKnowledgeProposal"
          @ignore-knowledge-proposal="chatStore.ignoreKnowledgeProposal"
          @tool-handoff-quiet-change="handleToolHandoffQuietChange"
          @tool-viewport-anchor-start="handleToolViewportAnchorStart"
          @tool-viewport-anchor-end="handleToolViewportAnchorEnd"
        >
        </ChatTranscript>
        <div v-if="showWelcomeState" class="chat-empty-overlay">
          <div class="empty-state">
            <div class="empty-icon">L</div>
            <div class="empty-title">Locus</div>
            <div class="empty-subtitle">{{ t("onboarding.welcome.subtitle") }}</div>
          </div>
        </div>
      </div>

    <div
      v-if="(pendingQuestion && !isViewingSubagent) || showBatchToolConfirmCard || showSingleToolConfirmCard || (isPlanDone && !isViewingSubagent) || (isPlanStreaming && !isViewingSubagent)"
      class="chat-pending-stack"
      @wheel="handleBottomPanelWheel"
    >
      <AskUserCard
        v-if="pendingQuestion && !isViewingSubagent"
        :question="pendingQuestion"
        @answer="handleQuestionAnswer"
      />

      <ToolConfirmBatchCard
        v-if="showBatchToolConfirmCard"
        :tool-confirms="pendingToolConfirms"
        @answer="emit('answerToolConfirm', $event.questionId, $event.answer)"
        @answer-many="emit('answerAllToolConfirms', $event.questionIds, $event.answer)"
      />
      <ToolConfirmCard
        v-else-if="showSingleToolConfirmCard"
        :tool-confirm="pendingToolConfirms[0]!"
        @answer="emit('answerToolConfirm', pendingToolConfirms[0]!.questionId, $event)"
      />

      <!-- Plan confirmation card after completion -->
      <div v-if="isPlanDone && !isViewingSubagent" class="plan-confirm-card">
        <span class="plan-confirm-text">{{ t('chat.plan.completed') }}</span>
        <div class="plan-confirm-actions">
          <BaseButton class="plan-confirm-btn ui-select-none" variant="primary" @click="handlePlanContinue">{{ t('chat.plan.continueImpl') }}</BaseButton>
          <BaseButton class="plan-confirm-btn ui-select-none" @click="chatStore.clearPendingPlan()">{{ t('chat.plan.dismiss') }}</BaseButton>
        </div>
      </div>

      <!-- Plan mode status bar -->
      <div v-if="isPlanStreaming && !isViewingSubagent" class="plan-status-bar">
        <svg class="plan-status-icon" viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
          <path d="M2 3.5A1.5 1.5 0 0 1 3.5 2h9A1.5 1.5 0 0 1 14 3.5v9a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5v-9zM5 5h6v1H5V5zm0 3h6v1H5V8zm0 3h4v1H5v-1z"/>
        </svg>
        <span>{{ t('chat.plan.planning') }}</span>
      </div>
    </div>

    <div
      v-if="!isViewingSubagent"
      class="input-area"
      :class="{
        'is-controls-collapsed': inputControlsCollapsed,
        'is-controls-switching': inputControlsSwitching,
      }"
    >
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
            :working-dir="workingDir"
            :is-unity-project="isUnityProject"
            :scan-phase="scanPhase"
            :last-scan-stats="lastScanStats"
            @start-scan="emit('startScan')"
            @install-plugin="emit('installPlugin')"
            @launch-unity-project="emit('launchUnityProject')"
          />
        </div>
        <div class="input-backdrop-action">
          <button
            v-if="!isViewingSubagent && hasPanelToggleRow"
            class="changes-toggle-btn ui-select-none"
            :class="{ 'is-active': chatChangesStore.currentPanelVisible }"
            type="button"
            :disabled="isStreaming"
            :aria-pressed="chatChangesStore.currentPanelVisible"
            @click="chatChangesStore.togglePanel()"
          >
            {{ t('chat.changes.toggle') }}
          </button>
        </div>
      </div>
      <RichChatInput
        ref="composerPanelRef"
        v-model="inputText"
        :selected-agent-id="selectedAgentId"
        :skills="skills"
        :placeholder="chatInputPlaceholder"
        :is-streaming="isStreaming"
        :send-label="t('common.send')"
        :cancel-label="t('common.cancel')"
        :compact="inputControlsCollapsed"
        :asset-ref-sync-key="composerAssetRefSyncKey"
        @send="handleComposerSend"
        @compact="emit('compact')"
        @clear="handleNewChatRequest"
        @cancel="emit('cancel')"
      >
        <template v-if="!inputControlsCollapsed" #footer-start>
          <ModelEffortSelector
            align="start"
            :models="models"
            :selected-id="selectedModelId"
            :effort="effort"
            :efforts="effortLevels"
            :effort-supported="effortSupported"
            :disabled="isStreaming"
            @select-model="emit('selectModel', $event)"
            @select-effort="emit('selectEffort', $event)"
          />
          <TokenUsageBar
            :token-usage="tokenUsage"
          />
        </template>
      </RichChatInput>
    </div>
    </div><!-- /chat-view -->

    <Teleport to="body">
      <div
        v-if="assetRefCtxMenu"
        class="asset-ref-ctx-backdrop"
        @click="closeAssetRefContextMenu"
        @contextmenu.prevent="closeAssetRefContextMenu"
      >
        <div
          ref="assetRefCtxMenuRef"
          class="asset-ref-ctx-menu"
          :style="assetRefCtxMenuStyle"
          @click.stop
        >
          <button
            v-if="assetRefContextIsKnowledge"
            type="button"
            class="asset-ref-ctx-item"
            @click="doAssetRefOpenInKnowledge"
          >
            {{ t("common.openInKnowledge") }}
          </button>
          <button
            v-else-if="assetRefContextCanOpenInEditor"
            type="button"
            class="asset-ref-ctx-item"
            @click="doAssetRefOpenInEditor"
          >
            {{ t("common.openInEditor") }}
          </button>
          <button type="button" class="asset-ref-ctx-item" @click="doAssetRefShowInFolder">
            {{ t("common.openInFileExplorer") }}
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
              {{ t("common.selectInUnity") }}
            </button>
          </template>
        </div>
      </div>
    </Teleport>

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
  flex: 1 1 0;
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

:deep(.sp-session-item:hover .sp-session-title),
:deep(.sp-session-item.active .sp-session-title) {
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
  border-top: 1px solid var(--border-color);
  background: var(--bg-color);
}

.input-area.is-controls-collapsed {
  padding-bottom: 14px;
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
  min-width: 0;
}

.chat-view.is-vertical-layout :deep(.chat-transcript-scroll.is-session) {
  padding: 14px 0;
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

.changes-toggle-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-family: inherit;
  font-weight: 600;
  height: 24px;
  min-height: 24px;
  line-height: 1;
  padding: 0 8px;
  cursor: pointer;
  box-shadow: none;
  white-space: nowrap;
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
  color: var(--text-color);
}

.changes-toggle-btn:focus-visible {
  outline: none;
}

.changes-toggle-btn:disabled {
  opacity: 0.48;
  cursor: not-allowed;
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

.asset-ref-ctx-backdrop {
  position: fixed;
  inset: 0;
  z-index: 9999;
}

.asset-ref-ctx-menu {
  position: fixed;
  z-index: 10000;
  box-sizing: border-box;
  min-width: min(184px, calc(100vw - 16px));
  max-width: calc(100vw - 16px);
  max-height: calc(100vh - 16px);
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 5px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--elevated-bg, var(--panel-bg));
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.24);
}

.asset-ref-ctx-item {
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 28px;
  padding: 0 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  line-height: 1;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
}

.asset-ref-ctx-item:hover:not(:disabled),
.asset-ref-ctx-item:focus-visible:not(:disabled) {
  background: var(--hover-bg);
}

.asset-ref-ctx-item:focus-visible {
  outline: none;
}

.asset-ref-ctx-item.disabled,
.asset-ref-ctx-item:disabled {
  color: var(--text-secondary);
  cursor: default;
  opacity: 0.48;
}

.asset-ref-ctx-sep {
  height: 1px;
  margin: 4px 0;
  background: var(--border-color);
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
