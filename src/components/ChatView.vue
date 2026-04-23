
<script setup lang="ts">
import { ref, nextTick, watch, computed, onMounted, onUnmounted } from "vue";
import { selectUnityAsset, openFileExternal, previewWorkspaceFile, showInFolder } from "../services/unity";
import type { WorkspaceFilePreview } from "../services/unity";
// undoPreview removed — undo UI moved to ChatChangesPanel
import type { ChatComposerSendPayload, ChatMessage, AgentInfo, TokenUsage, ModelOption, PendingQuestion, PendingToolConfirm, EffortLevel, SessionSummary, AssetDbScanEvent, ScanStats, ImageAttachment, SkillManifest, UserIntentMeta, SaveRawContextRequest, CodexTransportMode } from "../types";
import type { ToolCallDisplay } from "../types";
import ModelSelector from "./ModelSelector.vue";
import ThinkingSelector from "./ThinkingSelector.vue";
import SessionPanel from "./chat/SessionPanel.vue";
import ChatTranscript from "./chat/ChatTranscript.vue";
import RichChatInput from "./chat/RichChatInput.vue";
import TokenUsageBar from "./chat/TokenUsageBar.vue";
import AskUserCard from "./chat/AskUserCard.vue";
import ToolConfirmCard from "./chat/ToolConfirmCard.vue";
import ToolConfirmBatchCard from "./chat/ToolConfirmBatchCard.vue";
import FileDiffViewer from "./diff/FileDiffViewer.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";
import { refetchDiffByKey, createRequestToken, isTokenStale } from "../services/diff";
import { t } from "../i18n";
import { useChatChangesStore } from "../stores/chatChanges";
import { useChatStore } from "../stores/chat";
import { useUiStore } from "../stores/ui";
import {
  captureScrollAnchor,
  captureSessionScrollState,
  resolveSessionScrollTop,
  restoreScrollAnchor,
} from "../composables/chatScrollState";
import {
  createCoalescedScrollScheduler,
  createSettledScrollScheduler,
  shouldAutoScrollToBottom,
  shouldShowWaitingPlaceholder,
} from "../composables/chatViewStability";
import { forwardWheelToElement } from "../composables/chatWheelPassthrough";
import { useProjectStore } from "../stores/project";
import { canOpenInEditor } from "../composables/useHideMeta";
import { useDiffProgress } from "../composables/useDiffProgress";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import { matchesShortcut, useKeyboardShortcuts } from "../composables/useKeyboardShortcuts";
import {
  getChatSubmitModifierLabel,
  useChatInputSettings,
} from "../composables/useChatInputSettings";
import { logToolCollapseTrace, previewTraceText } from "../services/toolCollapseTrace";

const chatChangesStore = useChatChangesStore();
const chatStore = useChatStore();
const projectStore = useProjectStore();
const uiStore = useUiStore();
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

const showInlineDiff = computed(() =>
  !!chatChangesStore.inlineDiffPayload || chatChangesStore.inlineDiffLoading || !!chatChangesStore.inlineDiffError,
);
const hasPanelToggleRow = computed(() => chatChangesStore.currentFileCount > 0);

const chatDiffViewerRef = ref<InstanceType<typeof FileDiffViewer> | null>(null);
const chatDiffMode = ref<"unified" | "side-by-side">("unified");
const chatDiffTabOptions = computed(() => [
  { value: "semantic", label: t("diff.tabs.semantic") },
  { value: "text", label: t("diff.tabs.text") },
]);

function toggleChatDiffMode() {
  chatDiffMode.value = chatDiffMode.value === "unified" ? "side-by-side" : "unified";
}

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
  isStreaming: boolean;
  isThinking: boolean;
  hasThinking: boolean;
  thinkingText: string;
  thinkingDuration: number;
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
  scanPhase?: AssetDbScanEvent | null;
  lastScanStats?: ScanStats | null;
  isUnityProject?: boolean;
  skills?: SkillManifest[];
  streamingSessionIds?: Set<string>;
  undoableMessageIds?: Set<string>;
}>();


const emit = defineEmits<{
  send: [text: string, images: ImageAttachment[], overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null }];
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
}>();



const lightboxSrc = ref("");
function openLightbox(src: string) {
  lightboxSrc.value = src;
}
function closeLightbox() {
  lightboxSrc.value = "";
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
      handleFileRefClick(assetPath);
    }
    return;
  }
  const fileRef = target.closest(".md-file-ref") as HTMLElement | null;
  if (fileRef) {
    e.preventDefault();
    const filePath = fileRef.dataset.filePath;
    if (!filePath) return;
    handleFileRefClick(filePath);
  }
}

function handleFileRefClick(filePath: string) {
  if (canOpenInEditor(filePath)) {
    openFileExternal(filePath).catch((e: unknown) => console.warn("openFileExternal failed:", e));
    return;
  }
  if (props.unityConnected && filePath.startsWith("Assets/")) {
    selectUnityAsset(filePath).catch((e: unknown) => console.warn("selectUnityAsset failed:", e));
    return;
  }
  openFileExternal(filePath).catch((e: unknown) => console.warn("openFileExternal failed:", e));
}

function handleFolderRefClick(folderPath: string) {
  if (props.unityConnected && (folderPath.startsWith("Assets/") || folderPath.startsWith("Packages/"))) {
    selectUnityAsset(folderPath).catch((e: unknown) => console.warn("selectUnityAsset failed:", e));
    return;
  }
  showInFolder(folderPath).catch((e: unknown) => console.warn("showInFolder failed:", e));
}

// ── File preview hover state ──

const filePreviewState = ref<{
  anchor: HTMLElement;
  preview: WorkspaceFilePreview;
} | null>(null);

let fileHoverTimer: ReturnType<typeof setTimeout> | null = null;
let fileCloseTimer: ReturnType<typeof setTimeout> | null = null;
let currentFileRefEl: HTMLElement | null = null;

function handleFileRefMouseOver(e: MouseEvent) {
  const target = (e.target as HTMLElement).closest(".md-file-ref") as HTMLElement | null;

  if (!target) {
    // Mouse left a file-ref area — schedule close
    if (currentFileRefEl) {
      currentFileRefEl = null;
      if (fileHoverTimer) { clearTimeout(fileHoverTimer); fileHoverTimer = null; }
      scheduleFilePreviewClose();
    }
    return;
  }

  // Same element — no action needed
  if (target === currentFileRefEl) return;

  // New file-ref — cancel any pending close, start hover timer
  currentFileRefEl = target;
  cancelFilePreviewClose();
  if (fileHoverTimer) { clearTimeout(fileHoverTimer); fileHoverTimer = null; }

  fileHoverTimer = setTimeout(async () => {
    const filePath = target.dataset.filePath;
    if (!filePath) return;
    const line = target.dataset.fileLine ? Number(target.dataset.fileLine) : undefined;
    const token = createRequestToken();
    try {
      const preview = await previewWorkspaceFile(filePath, line);
      if (isTokenStale(token)) return;
      if (!preview.exists) return;
      filePreviewState.value = { anchor: target, preview };
    } catch (err) {
      if (!isTokenStale(token)) {
        console.warn("preview failed:", err);
      }
    }
  }, 300);
}

function scheduleFilePreviewClose() {
  if (fileCloseTimer) clearTimeout(fileCloseTimer);
  fileCloseTimer = setTimeout(() => {
    filePreviewState.value = null;
    currentFileRefEl = null;
  }, 200);
}

function cancelFilePreviewClose() {
  if (fileCloseTimer) { clearTimeout(fileCloseTimer); fileCloseTimer = null; }
}

function closeFilePreview() {
  if (fileHoverTimer) { clearTimeout(fileHoverTimer); fileHoverTimer = null; }
  if (fileCloseTimer) { clearTimeout(fileCloseTimer); fileCloseTimer = null; }
  createRequestToken(); // invalidate in-flight requests
  filePreviewState.value = null;
  currentFileRefEl = null;
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
  () => !!displayedStreamingText.value || props.activeToolCalls.length > 0
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
const isRestoringSessionView = ref(false);
const toolHandoffViewportQuiet = ref(false);
let suppressScrollCapture = false;
const displayedStreamingText = ref("");
let pendingStreamingText = "";
let streamingTextFlushTimer: ReturnType<typeof setTimeout> | null = null;
let sessionRestoreRevealFrame = 0;
const STREAMING_TEXT_RENDER_DELAY_MS = 80;
const STREAM_END_SCROLL_SETTLE_MS = 320;

function clearStreamingTextFlushTimer() {
  if (!streamingTextFlushTimer) return;
  clearTimeout(streamingTextFlushTimer);
  streamingTextFlushTimer = null;
}

function clearSessionRestoreRevealFrame() {
  if (!sessionRestoreRevealFrame) return;
  cancelAnimationFrame(sessionRestoreRevealFrame);
  sessionRestoreRevealFrame = 0;
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

function scrollToBottomNow(force = false) {
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

function restorePendingSessionScroll() {
  const targetSessionId = pendingRestoreSessionId.value;
  if (!targetSessionId || targetSessionId !== props.activeSessionId) return;

  clearSessionRestoreRevealFrame();
  nextTick(() => {
    const el = getMessagesElement();
    if (!el || pendingRestoreSessionId.value !== props.activeSessionId) return;

    const remembered = chatStore.getSessionScrollState(targetSessionId);
    restoreMessagesScrollState(remembered, targetSessionId);

    sessionRestoreRevealFrame = requestAnimationFrame(() => {
      sessionRestoreRevealFrame = 0;
      if (pendingRestoreSessionId.value !== targetSessionId || props.activeSessionId !== targetSessionId) return;
      isRestoringSessionView.value = false;
      pendingRestoreSessionId.value = null;
      pendingRestoreMessagesRef.value = null;
    });
  });
}

function onMessagesScroll() {
  if (suppressScrollCapture) return;
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  streamEndScrollScheduler.cancel();
  rememberScrollForSession();
  // Close file preview popover on scroll
  if (filePreviewState.value) closeFilePreview();
}

let transcriptResizeObserver: ResizeObserver | null = null;

function disconnectTranscriptResizeObserver() {
  transcriptResizeObserver?.disconnect();
  transcriptResizeObserver = null;
}

function connectTranscriptResizeObserver() {
  disconnectTranscriptResizeObserver();
  if (typeof ResizeObserver === "undefined") return;

  const scrollEl = getMessagesElement();
  const contentEl = getMessagesContentElement();
  if (!scrollEl && !contentEl) return;

  transcriptResizeObserver = new ResizeObserver(() => {
    if (suppressScrollCapture || toolHandoffViewportQuiet.value) return;
    if (pendingRestoreSessionId.value && pendingRestoreSessionId.value === props.activeSessionId) {
      restorePendingSessionScroll();
      return;
    }
    reconcileViewport();
  });

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
    streamEndScrollScheduler.cancel();
    preserveScrollAnchorScheduler.cancel();
    toolHandoffViewportQuiet.value = false;
    if (previousSessionId) {
      rememberScrollForSession(previousSessionId);
    }

    pendingRestoreSessionId.value = nextSessionId;
    pendingRestoreMessagesRef.value = props.messages;
    const shouldRestoreImmediately = !!nextSessionId && previousSessionId === null && !showWelcomeState.value;
    isRestoringSessionView.value = !!nextSessionId && !shouldRestoreImmediately;
    clearSessionRestoreRevealFrame();
    void restoreComposerDraft(nextSessionId ?? null);
    if (shouldRestoreImmediately) {
      restorePendingSessionScroll();
    }
  },
  { flush: "sync" },
);

watch(
  () => props.messages,
  (messages) => {
    if (!pendingRestoreSessionId.value || pendingRestoreSessionId.value !== props.activeSessionId) return;
    if (messages === pendingRestoreMessagesRef.value) return;
    isRestoringSessionView.value = true;
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
  emit("send", t("chat.plan.continueMessage"), []);
}

function handleComposerSend(payload: ChatComposerSendPayload) {
  if (chatStore.pendingPlanRun) {
    chatStore.clearPendingPlan();
  }

  emit("send", payload.text, payload.images, {
    displayText: payload.displayText,
    mode: payload.mode ?? undefined,
    userIntent: payload.userIntent ?? null,
  });
}

const STORAGE_KEY_SESSION_WIDTH = "locus:sessionPanelWidth";
const sessionPanelWidth = ref(220); // px
const isDraggingSession = ref(false);
const layoutRef = ref<HTMLElement | null>(null);
let releaseSessionSelectionLock: (() => void) | null = null;

function onSessionSplitterMouseDown(e: MouseEvent) {
  e.preventDefault();
  isDraggingSession.value = true;
  releaseSessionSelectionLock?.();
  releaseSessionSelectionLock = acquireSelectionLock();
  document.addEventListener("mousemove", onSessionSplitterMouseMove);
  document.addEventListener("mouseup", onSessionSplitterMouseUp);
}

function onSessionSplitterMouseMove(e: MouseEvent) {
  if (!isDraggingSession.value || !layoutRef.value) return;
  const rect = layoutRef.value.getBoundingClientRect();
  const x = e.clientX - rect.left;
  sessionPanelWidth.value = Math.max(140, Math.min(480, x));
}

function onSessionSplitterMouseUp() {
  isDraggingSession.value = false;
  document.removeEventListener("mousemove", onSessionSplitterMouseMove);
  document.removeEventListener("mouseup", onSessionSplitterMouseUp);
  releaseSessionSelectionLock?.();
  releaseSessionSelectionLock = null;
  try { localStorage.setItem(STORAGE_KEY_SESSION_WIDTH, String(sessionPanelWidth.value)); } catch {}
}

/* ── Tool Permission Mode (Auto / Ask) ── */
const toolPermMode = computed(() => chatStore.toolPermissionMode);

async function toggleToolPermMode() {
  await chatStore.toggleToolPermissionMode();
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
    const saved = localStorage.getItem(STORAGE_KEY_SESSION_WIDTH);
    if (saved) sessionPanelWidth.value = Math.max(140, Math.min(480, Number(saved)));
  } catch {}
  nextTick(() => {
    connectTranscriptResizeObserver();
  });
});

onUnmounted(() => {
  window.removeEventListener("keydown", onGlobalChatKeydown);
  rememberScrollForSession();
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  streamEndScrollScheduler.cancel();
  clearStreamingTextFlushTimer();
  clearSessionRestoreRevealFrame();
  disconnectTranscriptResizeObserver();
  document.removeEventListener("mousemove", onSessionSplitterMouseMove);
  document.removeEventListener("mouseup", onSessionSplitterMouseUp);
  releaseSessionSelectionLock?.();
  releaseSessionSelectionLock = null;
});
</script>

<template>
  <div class="chat-view-layout" ref="layoutRef" :class="{ 'dragging-session': isDraggingSession }">

    <!-- Inline diff panel — covers entire chat layout (session panel + chat area) -->
    <div v-if="showInlineDiff" class="diff-inline-panel">
      <template v-if="chatChangesStore.inlineDiffPayload">
        <div class="diff-inline-header">
          <button class="diff-back-btn ui-select-none" @click="chatChangesStore.closeInlineDiff()" title="Back">
            <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor"><path d="M7.78 12.53a.75.75 0 0 1-1.06 0L2.47 8.28a.75.75 0 0 1 0-1.06l4.25-4.25a.75.75 0 0 1 1.06 1.06L4.56 7.25h7.69a.75.75 0 0 1 0 1.5H4.56l3.22 3.22a.75.75 0 0 1 0 1.06z"/></svg>
          </button>
          <span class="diff-inline-status" :class="'status-' + (chatChangesStore.inlineDiffPayload.status ?? '').toLowerCase()">
            {{ chatChangesStore.inlineDiffPayload.status }}
          </span>
          <span v-if="chatChangesStore.inlineDiffPayload.oldPath" class="diff-inline-path" :title="chatChangesStore.inlineDiffPayload.oldPath + ' → ' + chatChangesStore.inlineDiffPayload.filePath">
            {{ chatChangesStore.inlineDiffPayload.oldPath }} → {{ chatChangesStore.inlineDiffPayload.filePath }}
          </span>
          <span v-else class="diff-inline-path" :title="chatChangesStore.inlineDiffPayload.filePath">
            {{ chatChangesStore.inlineDiffPayload.filePath }}
          </span>
          <span class="diff-inline-stats">
            <span class="stat-add">+{{ chatChangesStore.inlineDiffPayload.stats.additions }}</span>
            <span class="stat-del">-{{ chatChangesStore.inlineDiffPayload.stats.deletions }}</span>
          </span>
          <span class="diff-inline-actions">
            <BaseSegmented
              v-if="chatDiffViewerRef?.hasSemanticAndText"
              class="diff-inline-tab-group"
              size="sm"
              :model-value="chatDiffViewerRef.activeTab"
              :options="chatDiffTabOptions"
              @update:model-value="chatDiffViewerRef.activeTab = $event as 'semantic' | 'text'"
            />
            <BaseButton
              v-if="projectStore.unityConnected"
              class="diff-inline-action-btn ui-select-none"
              :title="t('common.selectInUnity')"
              @click="selectUnityAsset(chatChangesStore.inlineDiffPayload!.filePath)"
            >
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
              {{ t('common.selectInUnity') }}
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
            <BaseButton class="diff-inline-action-btn ui-select-none" @click="toggleChatDiffMode" :title="chatDiffMode === 'unified' ? 'Side-by-side' : 'Unified'">
              {{ chatDiffMode === "unified" ? "Side-by-side" : "Unified" }}
            </BaseButton>
          </span>
          <button class="diff-close-btn ui-select-none" @click="chatChangesStore.closeInlineDiff()">&times;</button>
        </div>
        <div class="diff-inline-body">
          <FileDiffViewer
            ref="chatDiffViewerRef"
            :payload="chatChangesStore.inlineDiffPayload"
            :mode="chatDiffMode"
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
      v-show="!showInlineDiff"
      :sessions="sessions"
      :active-session-id="activeSessionId"
      :unity-connected="unityConnected"
      :is-unity-project="isUnityProject"
      :scan-phase="scanPhase"
      :last-scan-stats="lastScanStats"
      :streaming-session-ids="streamingSessionIds"
      :session-panel-width="sessionPanelWidth"
      @select-session="emit('selectSession', $event)"
      @new-chat="handleNewChatRequest"
      @rename-session="(id: string, title: string) => emit('renameSession', id, title)"
      @archive-session="emit('archiveSession', $event)"
      @delete-session="emit('deleteSession', $event)"
      @start-scan="emit('startScan')"
      @save-raw-context="emit('saveRawContext', $event)"
    />

    <div v-show="!showInlineDiff" class="session-divider" @mousedown="onSessionSplitterMouseDown"></div>

    <div v-show="!showInlineDiff" class="chat-view">
      <div class="chat-main">
        <ChatTranscript
          ref="transcriptRef"
          :class="{ 'chat-transcript-restoring': isRestoringSessionView }"
          variant="session"
          :messages="messages"
          :streaming-text="displayedStreamingText"
          :is-streaming="isStreaming"
          :is-thinking="isThinking"
          :has-thinking="hasThinking"
          :thinking-duration="thinkingDuration"
          :active-tool-calls="activeToolCalls"
          user-label="You"
          assistant-label="Locus"
          :handoff-label="t('chat.transcript.handoff')"
          :waiting-label="t('chat.transcript.waiting')"
          :thinking-active-label="t('chat.transcript.thinking')"
          :thought-duration-label="t('chat.transcript.thoughtDuration', '{0}')"
          :thought-moment-label="t('chat.transcript.thoughtMoment')"
          enable-intent-badges
          show-user-images
          user-content-mode="asset"
          @scroll="onMessagesScroll"
          @content-click="handleContentClick"
          @content-mouseover="handleFileRefMouseOver"
          @content-mouseout="handleFileRefMouseOver"
          @open-thinking="emit('openThinking', $event)"
          @open-image="openLightbox"
          @apply-knowledge-proposal="chatStore.applyKnowledgeProposal"
          @ignore-knowledge-proposal="chatStore.ignoreKnowledgeProposal"
          @tool-handoff-quiet-change="handleToolHandoffQuietChange"
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
    >
      <RichChatInput
        ref="composerPanelRef"
        v-model="inputText"
        :selected-agent-id="selectedAgentId"
        :skills="skills"
        :placeholder="chatInputPlaceholder"
        :is-streaming="isStreaming"
        :send-label="t('common.send')"
        :cancel-label="t('common.cancel')"
        @send="handleComposerSend"
        @clear="handleNewChatRequest"
        @cancel="emit('cancel')"
      >
        <template #top-start>
          <ModelSelector
            :models="models"
            :selected-id="selectedModelId"
            :disabled="isStreaming"
            @select="emit('selectModel', $event)"
          />
          <ThinkingSelector
            v-if="effortSupported"
            :effort="effort"
            :efforts="effortLevels"
            :disabled="isStreaming"
            @select="emit('selectEffort', $event)"
          />
          <BaseButton
            class="perm-toggle-btn ui-select-none"
            :class="{ 'is-auto': toolPermMode === 'auto' }"
            :title="toolPermMode === 'auto' ? t('chat.perm.autoTitle') : t('chat.perm.askTitle')"
            @click="toggleToolPermMode"
          >
            <svg v-if="toolPermMode === 'auto'" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
              <path d="M13.78 4.22a.75.75 0 0 1 0 1.06l-7.25 7.25a.75.75 0 0 1-1.06 0L2.22 9.28a.75.75 0 0 1 1.06-1.06L6 10.94l6.72-6.72a.75.75 0 0 1 1.06 0z"/>
            </svg>
            <svg v-else viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
              <path d="M8 1a3.5 3.5 0 0 0-3.5 3.5v1H3.25A1.25 1.25 0 0 0 2 6.75v7A1.25 1.25 0 0 0 3.25 15h9.5A1.25 1.25 0 0 0 14 13.75v-7A1.25 1.25 0 0 0 12.75 5.5H11.5v-1A3.5 3.5 0 0 0 8 1zm-2 4.5v-1a2 2 0 1 1 4 0v1H6z"/>
            </svg>
            <span>{{ toolPermMode === 'auto' ? 'Auto' : 'Ask' }}</span>
          </BaseButton>
        </template>
        <template #top-end>
          <BaseButton
            v-if="!isViewingSubagent && hasPanelToggleRow"
            class="changes-toggle-btn ui-select-none"
            :variant="chatChangesStore.currentPanelVisible ? 'primary' : 'neutral'"
            :disabled="isStreaming"
            @click="chatChangesStore.togglePanel()"
          >
            <svg viewBox="0 0 16 16" fill="currentColor" width="11" height="11">
              <path d="M2 3.5A1.5 1.5 0 0 1 3.5 2h2A1.5 1.5 0 0 1 7 3.5v1A1.5 1.5 0 0 1 5.5 6h-2A1.5 1.5 0 0 1 2 4.5v-1zm0 8A1.5 1.5 0 0 1 3.5 10h2A1.5 1.5 0 0 1 7 11.5v1A1.5 1.5 0 0 1 5.5 14h-2A1.5 1.5 0 0 1 2 12.5v-1zM9.5 2h4a.5.5 0 0 1 0 1h-4a.5.5 0 0 1 0-1zm0 3h4a.5.5 0 0 1 0 1h-4a.5.5 0 0 1 0-1zm0 5h4a.5.5 0 0 1 0 1h-4a.5.5 0 0 1 0-1zm0 3h4a.5.5 0 0 1 0 1h-4a.5.5 0 0 1 0-1z"/>
            </svg>
            {{ t('chat.changes.toggle') }}
            <span class="changes-badge">{{ chatChangesStore.currentFileCount }}</span>
          </BaseButton>
        </template>
        <template #footer>
          <div class="footer-spacer"></div>
          <TokenUsageBar
            :token-usage="tokenUsage"
          />
        </template>
      </RichChatInput>
    </div>
    </div><!-- /chat-view -->

    <Transition name="lightbox">
      <div v-if="lightboxSrc" class="lightbox-overlay" @click="closeLightbox">
        <img :src="lightboxSrc" class="lightbox-img" @click.stop />
      </div>
    </Transition>
  </div><!-- /chat-view-layout -->
</template>

<style scoped>
.chat-view-layout {
  flex: 1;
  display: flex;
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.chat-view-layout.dragging-session {
  cursor: col-resize;
}

:deep(.session-panel) {
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

:deep(.sp-unity-status) {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--border-color);
  font-size: 11px;
  color: #ef4444;
}

:deep(.sp-unity-status.connected) {
  color: #22c55e;
}

:deep(.sp-unity-dot) {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #ef4444;
  flex-shrink: 0;
  opacity: 1;
}

:deep(.sp-unity-status.connected .sp-unity-dot) {
  background: #22c55e;
  opacity: 1;
}

:deep(.sp-unity-label) {
  font-weight: 500;
}

:deep(.sp-scan-status) {
  padding: 6px 14px;
  border-bottom: 1px solid var(--border-color);
  font-size: 11px;
  color: var(--text-secondary);
}

:deep(.sp-scan-row) {
  display: flex;
  align-items: center;
  gap: 5px;
}

:deep(.sp-scan-dot) {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--text-secondary);
  opacity: 0.4;
}

:deep(.sp-scan-dot.scanning) {
  background: var(--accent-color);
  opacity: 1;
  animation: sp-scan-pulse 1.2s ease-in-out infinite;
}

:deep(.sp-scan-dot.done) {
  background: #22c55e;
  opacity: 1;
}

:deep(.sp-scan-dot.error) {
  background: #e55;
  opacity: 1;
}

@keyframes sp-scan-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

:deep(.sp-scan-label) {
  flex: 1;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

:deep(.sp-scan-label.sp-scan-done) {
  color: #22c55e;
}

:deep(.sp-scan-label.sp-scan-idle) {
  opacity: 0.6;
}

:deep(.sp-scan-label.sp-scan-error) {
  color: #e55;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}


:deep(.sp-scan-btn) {
  flex-shrink: 0;
  padding: 1px 6px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 10px;
  cursor: pointer;
  transition: all 0.15s;
  box-shadow: none;
}

:deep(.sp-scan-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--text-secondary);
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

:deep(.sp-new-btn) {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-color);
  font-size: 16px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.15s;
  box-shadow: none;
  padding: 0;
}

:deep(.sp-new-btn:hover) {
  background: var(--hover-bg);
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
  flex: 1;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  position: relative;
  background: var(--msg-assistant-bg);
  contain: layout paint;
}

:deep(.chat-transcript-scroll.chat-transcript-restoring) {
  visibility: hidden;
}

.chat-main {
  position: relative;
  flex: 1;
  min-height: 0;
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
  padding: 12px 48px 24px;
  border-top: 1px solid var(--border-color);
  background: var(--bg-color);
}

.chat-pending-stack {
  min-width: 0;
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

.perm-toggle-btn {
  gap: 4px;
  font-size: 12px;
  font-family: inherit;
  font-weight: 500;
  white-space: nowrap;
  min-height: 28px;
  padding: 0 8px;
}

.perm-toggle-btn.is-auto {
  background: color-mix(in srgb, var(--accent-soft) 72%, var(--panel-bg) 28%);
  border-color: color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  color: color-mix(in srgb, var(--accent-color) 88%, var(--text-color) 12%);
}

.perm-toggle-btn.is-auto:hover:not(:disabled) {
  background: color-mix(in srgb, var(--accent-soft) 84%, var(--panel-bg) 16%);
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  color: var(--accent-color);
  filter: none;
}

.changes-toggle-btn {
  gap: 4px;
  font-size: 12px;
  font-family: inherit;
  font-weight: 500;
  min-height: 28px;
  padding: 0 10px;
}

.changes-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 14px;
  height: 14px;
  padding: 0 3px;
  border-radius: 7px;
  background: color-mix(in srgb, currentColor 18%, transparent);
  color: inherit;
  font-size: 9px;
  font-weight: 600;
  line-height: 1;
}

.footer-spacer {
  flex: 1;
}

:deep(.token-usage-group) {
  display: flex;
  align-items: center;
  gap: 10px;
  cursor: default;
  justify-content: flex-end;
  white-space: nowrap;
}

:deep(.context-usage) {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  color: var(--text-secondary);
}

:deep(.context-label) {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.3px;
  opacity: 0.6;
}

:deep(.context-bar-track) {
  width: 48px;
  height: 4px;
  border-radius: 2px;
  background: var(--border-color);
  overflow: hidden;
}

:deep(.context-bar-fill) {
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s ease, background 0.3s ease;
}

:deep(.context-text) {
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
}

:deep(.context-sep) {
  opacity: 0.4;
  margin: 0 1px;
}

:deep(.token-price) {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--text-secondary);
}

:deep(.price-label) {
  text-transform: uppercase;
  letter-spacing: 0.3px;
  opacity: 0.5;
}

:deep(.price-total) {
  font-variant-numeric: tabular-nums;
  color: var(--text-color);
  opacity: 0.8;
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
.diff-back-btn {
  background: none;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  padding: 2px 8px;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 14px;
}
.diff-back-btn:hover {
  color: var(--text-color);
  border-color: var(--text-secondary);
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
