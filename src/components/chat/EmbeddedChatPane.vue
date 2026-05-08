<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, useSlots, watch } from "vue";
import type {
  ChatComposerSendPayload,
  ChatMessage,
  PendingQuestion,
  PendingToolConfirm,
  SkillManifest,
  ToolCallDisplay,
  AssistantRenderPart,
} from "../../types";
import AskUserCard from "./AskUserCard.vue";
import ToolConfirmCard from "./ToolConfirmCard.vue";
import ToolConfirmBatchCard from "./ToolConfirmBatchCard.vue";
import ChatTranscript from "./ChatTranscript.vue";
import RichChatInput from "./RichChatInput.vue";
import { forwardWheelToElement } from "../../composables/chatWheelPassthrough";
import {
  captureScrollAnchor,
  captureLiveScrollAnchor,
  captureSessionScrollState,
  resolveSessionScrollTop,
  restoreLiveScrollAnchor,
  restoreScrollAnchor,
  type LiveScrollAnchorSnapshot,
  type SessionScrollState,
} from "../../composables/chatScrollState";
import {
  createCoalescedScrollScheduler,
  createSettledScrollScheduler,
  shouldAutoScrollToBottom,
} from "../../composables/chatViewStability";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../../composables/resizeObserver";
import { t } from "../../i18n";

interface MetaRow {
  label: string;
  value: string;
}

const props = withDefaults(defineProps<{
  title?: string;
  subtitle?: string;
  metaRows?: MetaRow[];
  messages: ChatMessage[];
  streamingText: string;
  streamingTextOrder?: number;
  thinkingText?: string;
  thinkingOrder?: number;
  isStreaming: boolean;
  isCompacting?: boolean;
  isThinking: boolean;
  thinkingDuration?: number;
  liveRenderParts?: AssistantRenderPart[];
  activeToolCalls: ToolCallDisplay[];
  pendingQuestion?: PendingQuestion | null;
  pendingToolConfirms?: PendingToolConfirm[];
  toolConfirmLayoutKey?: string | null;
  inputValue: string;
  placeholder?: string;
  emptyTitle?: string;
  emptyHint?: string;
  errorMessage?: string | null;
  disabled?: boolean;
  sendLabel?: string;
  cancelLabel?: string;
  userLabel?: string;
  assistantLabel?: string;
  thinkingLabel?: string;
  waitingLabel?: string;
  compactingLabel?: string;
  compactedLabel?: string;
  thoughtDurationLabel?: string;
  thoughtMomentLabel?: string;
  runningLabel?: string;
  selectedAgentId?: string;
  skills?: SkillManifest[];
  enableIntentBadges?: boolean;
  showUserImages?: boolean;
  userContentMode?: "plain" | "asset";
}>(), {
  subtitle: "",
  metaRows: () => [],
  thinkingText: "",
  thinkingDuration: 0,
  isCompacting: false,
  pendingQuestion: null,
  pendingToolConfirms: () => [],
  toolConfirmLayoutKey: null,
  placeholder: "",
  emptyTitle: "",
  emptyHint: "",
  errorMessage: null,
  disabled: false,
  sendLabel: "",
  cancelLabel: "",
  userLabel: "",
  assistantLabel: "Locus",
  thinkingLabel: "",
  waitingLabel: "",
  compactingLabel: "",
  compactedLabel: "",
  thoughtDurationLabel: "",
  thoughtMomentLabel: "",
  runningLabel: "",
  selectedAgentId: "",
  skills: () => [],
  enableIntentBadges: false,
  showUserImages: false,
  userContentMode: "plain",
});

const emit = defineEmits<{
  (e: "update:inputValue", value: string): void;
  (e: "send", payload: ChatComposerSendPayload): void;
  (e: "cancel"): void;
  (e: "clear"): void;
  (e: "answerQuestion", value: string): void;
  (e: "answerToolConfirm", questionId: string, answer: string): void;
  (e: "answerAllToolConfirms", questionIds: string[], answer: string): void;
  (e: "applyKnowledgeProposal", proposalId: string): void;
  (e: "ignoreKnowledgeProposal", proposalId: string): void;
}>();

const slots = useSlots();
const transcriptRef = ref<InstanceType<typeof ChatTranscript> | null>(null);
const hasHeader = computed(() => !!props.title || !!props.subtitle || !!slots["header-actions"]);
const hasComposerStart = computed(() => !!slots["composer-start"]);
const hasComposerActions = computed(() => !!slots["composer-actions"]);
const effectiveSendLabel = computed(() => props.sendLabel || t("common.send"));
const effectiveCancelLabel = computed(() => props.cancelLabel || t("common.cancel"));
const effectiveUserLabel = computed(() => props.userLabel || t("chat.embedded.user"));
const effectiveThinkingLabel = computed(() => props.thinkingLabel || t("chat.embedded.thinking"));
const effectiveWaitingLabel = computed(() => props.waitingLabel || props.runningLabel || t("chat.embedded.running"));
const effectiveCompactingLabel = computed(() => props.compactingLabel || t("chat.transcript.compacting"));
const effectiveCompactedLabel = computed(() => props.compactedLabel || t("chat.transcript.compacted"));
const effectiveThoughtDurationLabel = computed(() =>
  props.thoughtDurationLabel || t("chat.transcript.thoughtDuration", "{0}"),
);
const effectiveThoughtMomentLabel = computed(() =>
  props.thoughtMomentLabel || t("chat.transcript.thoughtMoment"),
);
const viewportStates = new Map<string, SessionScrollState>();
let suppressScrollCapture = false;
let transcriptResizeObserver: ResizeObserverHandle | null = null;
const toolHandoffViewportQuiet = ref(false);
let activeToolViewportAnchor: LiveScrollAnchorSnapshot | null = null;
let toolViewportAnchorFrame = 0;
const STREAM_END_SCROLL_SETTLE_MS = 320;

function updateInput(value: string) {
  emit("update:inputValue", value);
}

function getViewportStateKey(key = props.toolConfirmLayoutKey) {
  return key?.trim() || "__embedded__";
}

function getTranscriptElement() {
  return transcriptRef.value?.getScrollElement() ?? null;
}

function getTranscriptContentElement() {
  return transcriptRef.value?.getContentElement?.() ?? null;
}

function readTranscriptMetrics(el: HTMLElement) {
  return {
    scrollTop: el.scrollTop,
    clientHeight: el.clientHeight,
    scrollHeight: el.scrollHeight,
  };
}

function captureViewportState(el: HTMLElement): SessionScrollState {
  return captureSessionScrollState(readTranscriptMetrics(el), captureScrollAnchor(el));
}

function rememberViewportState(key = getViewportStateKey()) {
  const el = getTranscriptElement();
  if (!el) return;
  viewportStates.set(key, captureViewportState(el));
}

function runProgrammaticScrollUpdate(update: (el: HTMLElement) => void, key = getViewportStateKey()) {
  const el = getTranscriptElement();
  if (!el) return;

  suppressScrollCapture = true;
  update(el);
  viewportStates.set(key, captureViewportState(el));

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
  const el = getTranscriptElement();
  if (!anchorState || !el) return false;
  if (!el.contains(anchorState.anchor)) {
    clearToolViewportAnchor();
    return false;
  }

  suppressScrollCapture = true;
  const restored = restoreLiveScrollAnchor(el, anchorState);
  if (restored) {
    viewportStates.set(getViewportStateKey(), captureViewportState(el));
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
  const el = getTranscriptElement();
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

function scrollToBottomNow(force = false) {
  const el = getTranscriptElement();
  if (!el) return;

  const remembered = viewportStates.get(getViewportStateKey()) ?? null;
  if (!shouldAutoScrollToBottom({ force, metrics: readTranscriptMetrics(el), remembered })) {
    return;
  }

  runProgrammaticScrollUpdate((element) => {
    element.scrollTop = resolveSessionScrollTop(readTranscriptMetrics(element), { mode: "bottom" });
  });
}

const scrollToBottomScheduler = createCoalescedScrollScheduler((force) => {
  nextTick(() => {
    scrollToBottomNow(force);
  });
});

const preserveScrollAnchorScheduler = createCoalescedScrollScheduler(() => {
  nextTick(() => {
    const remembered = viewportStates.get(getViewportStateKey()) ?? null;
    if (!remembered || remembered.mode === "bottom") return;

    const el = getTranscriptElement();
    if (!el) return;

    const nextScrollTop = resolveSessionScrollTop(readTranscriptMetrics(el), remembered);
    runProgrammaticScrollUpdate((element) => {
      if (!restoreScrollAnchor(element, remembered)) {
        element.scrollTop = nextScrollTop;
      }
    });
  });
});

const streamEndScrollScheduler = createSettledScrollScheduler(
  () => scrollToBottom(true),
  STREAM_END_SCROLL_SETTLE_MS,
);

function handleToolHandoffQuietChange(quiet: boolean) {
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

function scrollToBottom(force = false) {
  scrollToBottomScheduler.schedule(force);
}

function preserveScrollAnchor() {
  preserveScrollAnchorScheduler.schedule();
}

function reconcileViewport(forceBottom = false) {
  if (toolHandoffViewportQuiet.value) return;
  if (restoreToolViewportAnchor()) return;
  const el = getTranscriptElement();
  if (!el) return;

  const remembered = viewportStates.get(getViewportStateKey()) ?? null;
  if (shouldAutoScrollToBottom({ force: forceBottom, metrics: readTranscriptMetrics(el), remembered })) {
    scrollToBottom(forceBottom);
    return;
  }

  preserveScrollAnchor();
}

function restoreViewportStateForKey(key = getViewportStateKey()) {
  const remembered = viewportStates.get(key) ?? null;
  if (!remembered) {
    scrollToBottom(true);
    return;
  }

  const el = getTranscriptElement();
  if (!el) return;

  const nextScrollTop = resolveSessionScrollTop(readTranscriptMetrics(el), remembered);
  runProgrammaticScrollUpdate((element) => {
    if (!restoreScrollAnchor(element, remembered)) {
      element.scrollTop = nextScrollTop;
    }
  }, key);
}

function handleTranscriptScroll() {
  if (suppressScrollCapture) return;
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  streamEndScrollScheduler.cancel();
  rememberViewportState();
}

function disconnectTranscriptResizeObserver() {
  transcriptResizeObserver?.disconnect();
  transcriptResizeObserver = null;
}

function connectTranscriptResizeObserver() {
  disconnectTranscriptResizeObserver();
  if (typeof ResizeObserver === "undefined") return;

  const scrollEl = getTranscriptElement();
  const contentEl = getTranscriptContentElement();
  if (!scrollEl && !contentEl) return;

  transcriptResizeObserver = createAnimationFrameResizeObserver(() => {
    if (suppressScrollCapture || toolHandoffViewportQuiet.value) return;
    if (restoreToolViewportAnchor()) return;
    reconcileViewport();
  });
  if (!transcriptResizeObserver) return;

  if (scrollEl) {
    transcriptResizeObserver.observe(scrollEl);
  }
  if (contentEl && contentEl !== scrollEl) {
    transcriptResizeObserver.observe(contentEl);
  }
}

function handleBottomPanelWheel(event: WheelEvent) {
  forwardWheelToElement(event, getTranscriptElement());
}

const keepBatchToolConfirmLayout = ref(false);

watch(
  () => props.toolConfirmLayoutKey ?? "",
  (nextKey, previousKey) => {
    clearToolViewportAnchor();
    toolHandoffViewportQuiet.value = false;
    rememberViewportState(getViewportStateKey(previousKey));
    preserveScrollAnchorScheduler.cancel();
    streamEndScrollScheduler.cancel();
    nextTick(() => {
      restoreViewportStateForKey(getViewportStateKey(nextKey));
      connectTranscriptResizeObserver();
    });
  },
  { flush: "pre" },
);

watch(
  () => [props.toolConfirmLayoutKey ?? "", props.pendingToolConfirms.map((item) => item.questionId).join(":")],
  ([layoutKey], previous = []) => {
    const [prevLayoutKey] = previous;
    const count = props.pendingToolConfirms.length;
    if (layoutKey !== prevLayoutKey) {
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
  props.pendingToolConfirms.length > 0
  && (keepBatchToolConfirmLayout.value || props.pendingToolConfirms.length > 1),
);

const showSingleToolConfirmCard = computed(() =>
  props.pendingToolConfirms.length === 1
  && !showBatchToolConfirmCard.value,
);

watch(
  () => props.messages,
  (messages, previous) => {
    if (messages === previous) return;
    reconcileViewport();
  },
  { flush: "post" },
);

watch(() => props.messages.length, () => reconcileViewport());
watch(() => props.streamingText, () => reconcileViewport());
watch(() => props.thinkingText, () => reconcileViewport());
watch(() => props.isThinking, () => reconcileViewport());
watch(() => props.activeToolCalls, () => reconcileViewport(), { deep: true });
watch(
  () => props.isStreaming,
  (nextStreaming, previousStreaming) => {
    if (nextStreaming) {
      streamEndScrollScheduler.cancel();
      return;
    }
    if (previousStreaming) {
      if (toolHandoffViewportQuiet.value) return;
      const el = getTranscriptElement();
      const remembered = el ? viewportStates.get(getViewportStateKey()) ?? null : null;
      if (el && !shouldAutoScrollToBottom({ metrics: readTranscriptMetrics(el), remembered })) {
        preserveScrollAnchor();
        return;
      }
      streamEndScrollScheduler.schedule();
    }
  },
);
watch(() => props.pendingQuestion?.questionId ?? "", (questionId) => {
  if (questionId) reconcileViewport();
});
watch(() => props.pendingToolConfirms.map((item) => item.questionId).join(":"), (value) => {
  if (value) reconcileViewport();
});
watch(
  () => props.messages.length,
  (length) => {
    if (length === 0 && !props.isStreaming) {
      viewportStates.set(getViewportStateKey(), { mode: "bottom" });
    }
  },
);

onMounted(() => {
  nextTick(() => {
    restoreViewportStateForKey();
    connectTranscriptResizeObserver();
  });
});

onUnmounted(() => {
  rememberViewportState();
  scrollToBottomScheduler.cancel();
  preserveScrollAnchorScheduler.cancel();
  streamEndScrollScheduler.cancel();
  clearToolViewportAnchor();
  disconnectTranscriptResizeObserver();
});
</script>

<template>
  <div class="embedded-chat-pane">
    <div v-if="hasHeader" class="embedded-chat-header">
      <div class="embedded-chat-heading">
        <div class="embedded-chat-title">{{ title }}</div>
        <div v-if="subtitle" class="embedded-chat-subtitle">{{ subtitle }}</div>
      </div>
      <div class="embedded-chat-header-actions">
        <slot name="header-actions" />
      </div>
    </div>

    <div v-if="metaRows.length > 0" class="embedded-chat-context">
      <div v-for="row in metaRows" :key="row.label" class="embedded-chat-context-row">
        <span class="embedded-chat-context-label">{{ row.label }}</span>
        <span class="embedded-chat-context-value" :title="row.value">{{ row.value }}</span>
      </div>
    </div>

    <div v-if="errorMessage" class="embedded-chat-error">{{ errorMessage }}</div>

    <ChatTranscript
      ref="transcriptRef"
      variant="embedded"
      :session-key="getViewportStateKey()"
      :messages="messages"
      :streaming-text="streamingText"
      :streaming-text-order="streamingTextOrder"
      :is-streaming="isStreaming"
      :is-compacting="isCompacting"
      :is-thinking="isThinking"
      :thinking-text="thinkingText"
      :thinking-order="thinkingOrder"
      :thinking-duration="thinkingDuration"
      :live-render-parts="liveRenderParts"
      :active-tool-calls="activeToolCalls"
      :empty-title="emptyTitle"
      :empty-hint="emptyHint"
      :user-label="effectiveUserLabel"
      :assistant-label="assistantLabel"
      :waiting-label="effectiveWaitingLabel"
      :compacting-label="effectiveCompactingLabel"
      :compacted-label="effectiveCompactedLabel"
      :thinking-active-label="effectiveThinkingLabel"
      :thought-duration-label="effectiveThoughtDurationLabel"
      :thought-moment-label="effectiveThoughtMomentLabel"
      :enable-intent-badges="enableIntentBadges"
      :show-user-images="showUserImages"
      :user-content-mode="userContentMode"
      @scroll="handleTranscriptScroll"
      @apply-knowledge-proposal="emit('applyKnowledgeProposal', $event)"
      @ignore-knowledge-proposal="emit('ignoreKnowledgeProposal', $event)"
      @tool-handoff-quiet-change="handleToolHandoffQuietChange"
      @tool-viewport-anchor-start="handleToolViewportAnchorStart"
      @tool-viewport-anchor-end="handleToolViewportAnchorEnd"
    />

    <div class="embedded-chat-bottom">
      <div
        v-if="pendingQuestion || showBatchToolConfirmCard || showSingleToolConfirmCard"
        class="embedded-chat-panels"
        @wheel="handleBottomPanelWheel"
      >
        <AskUserCard
          v-if="pendingQuestion"
          :question="pendingQuestion"
          @answer="emit('answerQuestion', $event)"
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
      </div>

      <RichChatInput
        :model-value="inputValue"
        :selected-agent-id="selectedAgentId"
        :skills="skills"
        :placeholder="placeholder"
        :disabled="disabled"
        :is-streaming="isStreaming"
        :send-label="effectiveSendLabel"
        :cancel-label="effectiveCancelLabel"
        @update:model-value="updateInput"
        @send="emit('send', $event)"
        @clear="emit('clear')"
        @cancel="emit('cancel')"
      >
        <template v-if="hasComposerStart" #top-start>
          <slot name="composer-start" />
        </template>
        <template v-if="hasComposerActions" #top-end>
          <slot name="composer-actions" />
        </template>
      </RichChatInput>
    </div>
  </div>
</template>

<style scoped>
.embedded-chat-pane {
  height: 100%;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: color-mix(in srgb, var(--sidebar-bg) 76%, var(--panel-bg));
}

.embedded-chat-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 90%, var(--panel-bg));
}

.embedded-chat-heading {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.embedded-chat-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.embedded-chat-subtitle {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.45;
}

.embedded-chat-header-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  flex-shrink: 0;
  min-height: 28px;
}

.embedded-chat-context {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--sidebar-bg) 26%);
}

.embedded-chat-context-row {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr);
  gap: 10px;
  align-items: start;
}

.embedded-chat-context-label {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.45;
}

.embedded-chat-context-value {
  font-size: 11px;
  color: var(--text-color);
  line-height: 1.45;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--font-mono-identifier);
}

.embedded-chat-error {
  padding: 7px 12px;
  border-bottom: 1px solid var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
}

.embedded-chat-bottom {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px 12px;
  border-top: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 92%, var(--panel-bg));
}

.embedded-chat-panels {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
}

:deep(.ask-user-card) {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--sidebar-bg) 14%);
}

:deep(.knowledge-confirm-card) {
  margin: 0;
}

:deep(.tool-confirm-batch-card) {
  margin: 0;
}

:deep(.ask-question) {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.5;
}

:deep(.ask-options) {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

:deep(.ask-option-btn) {
  align-items: flex-start;
  justify-content: flex-start;
  flex-direction: column;
  gap: 4px;
  padding-block: 8px;
}

:deep(.ask-option-label) {
  font-size: 12px;
  font-weight: 600;
  color: inherit;
}

:deep(.ask-option-desc) {
  font-size: 11px;
  color: var(--text-secondary);
  text-align: left;
  white-space: normal;
  line-height: 1.5;
}

:deep(.ask-custom) {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

:deep(.ask-custom-label) {
  font-size: 11px;
  color: var(--text-secondary);
}

:deep(.ask-custom-input-row) {
  display: flex;
  gap: 8px;
}

:deep(.ask-custom-input) {
  flex: 1;
  min-width: 0;
  min-height: 32px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--panel-bg));
  color: var(--text-color);
  font: inherit;
  box-shadow: none;
}

:deep(.ask-custom-input:focus) {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color));
}

:deep(.ask-custom-send) {
  min-width: 40px;
  padding-inline: 0;
}

:deep(.tool-confirm-card) {
  gap: 12px;
}

:deep(.tool-confirm-header) {
  display: flex;
  align-items: center;
  gap: 6px;
}

:deep(.tool-confirm-icon) {
  color: var(--accent-color);
}

:deep(.tool-confirm-title) {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

:deep(.tool-confirm-body) {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

:deep(.tool-confirm-name) {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

:deep(.tool-confirm-args) {
  margin: 0;
  padding: 10px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.55;
  font-family: var(--font-mono-block);
  white-space: pre-wrap;
  word-break: break-word;
}

:deep(.tool-confirm-actions) {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

:deep(.tool-confirm-btn) {
  min-width: 72px;
}

</style>
