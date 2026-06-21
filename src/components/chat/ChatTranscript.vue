<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, useSlots, watch } from "vue";
import { FileText } from "lucide";
import type { AssetRefAttachment, AssistantRenderPart, ChatMessage, ImageAttachment, ToolCallDisplay, ToolCallInfo, UserIntentMeta } from "../../types";
import { t } from "../../i18n";
import {
  collectPendingContinuationToolSegmentItemIds,
  shouldShowAssistantContinuation,
  shouldShowWaitingPlaceholder,
  type PendingContinuationRenderSegment,
} from "../../composables/chatViewStability";
import {
  buildMessageToolCalls,
  cloneToolCallMatchState,
  collectToolCallDisplayIds,
  collectToolCallDisplayIdMatchState,
  collectToolCallDisplayMatchState,
  areToolCallDisplaysCoveredByMatchState,
  filterToolCallsByConsumableMatchState,
  firstToolCallRenderOrder,
  getToolCallInfoFingerprint,
  hasVisibleTextPartAfterToolCalls,
  lastToolCallRenderOrder,
  mergeSequentialAssistantToolCalls,
  mergeToolCallDisplaysWithoutDuplicates,
  mergeToolCallMatchStates,
  resolveToolCallInfosForRender,
  summarizeToolCallBatch,
} from "../../composables/toolCallBatches";
import type { ToolCallMatchState } from "../../composables/toolCallBatches";
import { collectRenderedToolCalls, planPromotedToolVisibility } from "../../composables/toolRenderPlan";
import {
  assertCanonicalRenderParts,
  compareAssistantRenderParts,
  synthesizeLegacyRenderParts,
} from "../../composables/assistantRenderParts";
import { useThrottledLiveRenderParts } from "../../composables/streamingRenderThrottle";
import { useDisplaySettings } from "../../composables/useDisplaySettings";
import { parseChatAssetRefs } from "../../composables/chatAssetRefs";
import {
  displayUserMessageContent,
  userMessageConsoleEntries,
  userMessageLocalFileEntries,
  type UserConsoleEntryDisplay,
  type UserLocalFileEntryDisplay,
} from "../../composables/chatUserMessageDisplay";
import { logToolCollapseTrace, previewTraceText } from "../../services/toolCollapseTrace";
import {
  traceToolBlockLayoutChange,
  traceTranscriptPaintOcclusion,
} from "../../services/layoutDiagnostics";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import ToolCallCollection from "../ToolCallCollection.vue";
import ToolCallBlock from "../ToolCallBlock.vue";
import KnowledgeProposalCard from "./KnowledgeProposalCard.vue";
import ChatWaitingIndicator from "./ChatWaitingIndicator.vue";
import AssetChip from "../AssetChip.vue";
import LucideIcon from "../icons/LucideIcon.vue";

type TranscriptVariant = "session" | "embedded";
type UserContentMode = "plain" | "asset";

interface MessageGroup {
  id: string;
  role: "user" | "assistant";
  items: MessageRenderItem[];
}

interface MessageRenderItem {
  id: string;
  order: number;
  message: ChatMessage;
  attachedKnowledgeProposals: ChatMessage[];
  hidden: boolean;
  displayToolCalls?: ToolCallInfo[];
  displayToolCallsBeforeContent?: ToolCallInfo[];
  displayToolCallsAfterContent?: ToolCallInfo[];
}

interface ToolCallHandoffState {
  renderKey: string;
  createdAt: number;
  toolCalls: ToolCallDisplay[];
  toolCallIds: Set<string>;
  toolCallMatchState: ToolCallMatchState;
  collapseCandidateToolCalls: ToolCallDisplay[];
  willAutoCollapse: boolean;
  collapseArmed: boolean;
  collapseFinished: boolean;
}

interface PromotedHistoryToolCallsState {
  itemIds: Set<string>;
  toolCallIds: Set<string>;
  toolCallMatchState: ToolCallMatchState;
  toolCalls: ToolCallDisplay[];
}

const props = withDefaults(defineProps<{
  messages: ChatMessage[];
  streamingText: string;
  streamingTextOrder?: number;
  isStreaming: boolean;
  isCompacting?: boolean;
  isThinking: boolean;
  hasThinking?: boolean;
  thinkingText?: string;
  thinkingOrder?: number;
  thinkingDuration?: number;
  liveRenderParts?: AssistantRenderPart[];
  activeToolCalls: ToolCallDisplay[];
  variant?: TranscriptVariant;
  emptyTitle?: string;
  emptyHint?: string;
  userLabel?: string;
  assistantLabel?: string;
  handoffLabel?: string;
  waitingLabel?: string;
  compactingLabel?: string;
  compactedLabel?: string;
  thinkingActiveLabel?: string;
  thoughtDurationLabel?: string;
  thoughtMomentLabel?: string;
  enableIntentBadges?: boolean;
  showUserImages?: boolean;
  userContentMode?: UserContentMode;
  sessionKey?: string | null;
  selectedMessageId?: string | null;
}>(), {
  variant: "embedded",
  hasThinking: undefined,
  streamingTextOrder: 0,
  isCompacting: false,
  thinkingText: "",
  thinkingOrder: 0,
  thinkingDuration: 0,
  liveRenderParts: () => [],
  emptyTitle: "",
  emptyHint: "",
  userLabel: "User",
  assistantLabel: "Locus",
  handoffLabel: "Handoff",
  waitingLabel: "Waiting for response…",
  compactingLabel: "Compacting context…",
  compactedLabel: "Context compacted",
  thinkingActiveLabel: "Thinking…",
  thoughtDurationLabel: "Thought for {0}s",
  thoughtMomentLabel: "Thought for a moment",
  enableIntentBadges: false,
  showUserImages: false,
  userContentMode: "plain",
  sessionKey: null,
  selectedMessageId: null,
});

const emit = defineEmits<{
  (e: "applyKnowledgeProposal", proposalId: string): void;
  (e: "ignoreKnowledgeProposal", proposalId: string): void;
  (e: "openThinking", content: string): void;
  (e: "openImage", src: string): void;
  (e: "scroll", event: Event): void;
  (e: "contentClick", event: MouseEvent): void;
  (e: "contentContextmenu", event: MouseEvent): void;
  (e: "userScrollIntent", event: Event): void;
  (e: "toolHandoffQuietChange", quiet: boolean): void;
  (e: "toolViewportAnchorStart", anchor: HTMLElement): void;
  (e: "toolViewportAnchorEnd", anchor: HTMLElement): void;
}>();

const scrollRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);
const { state: displaySettings } = useDisplaySettings();
const slots = useSlots();

defineExpose({
  getScrollElement: () => scrollRef.value,
  getContentElement: () => contentRef.value,
  scrollToBottom() {
    const element = scrollRef.value;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
  },
});

const TOOL_HANDOFF_MIN_VISIBLE_MS = 160;

function buildIntentBadges(
  intent: Pick<UserIntentMeta, "mode" | "skills"> | null | undefined,
) {
  const badges: Array<{ key: string; label: string; kind: "plan" | "skill" }> = [];
  if (!intent) return badges;

  if (intent.mode === "plan") {
    badges.push({ key: "plan", label: "Plan", kind: "plan" });
  }

  for (const skill of intent.skills || []) {
    badges.push({
      key: `${skill.source}:${skill.dirName}`,
      label: skill.name,
      kind: "skill",
    });
  }

  return badges;
}

function messageIntentBadges(message: ChatMessage) {
  return buildIntentBadges(message.intentMeta ?? null);
}

function userMessageDisplayContent(message: ChatMessage) {
  return displayUserMessageContent(message.content);
}

function userMessageDisplaySegments(message: ChatMessage) {
  const content = userMessageDisplayContent(message);
  return props.userContentMode === "asset"
    ? parseChatAssetRefs(content)
    : [{ type: "text" as const, value: content }];
}

function messageAssetRefs(message: ChatMessage): AssetRefAttachment[] {
  return message.assetRefs ?? [];
}

function messageConsoleEntries(message: ChatMessage): UserConsoleEntryDisplay[] {
  return userMessageConsoleEntries(message.content);
}

function messageLocalFileEntries(message: ChatMessage): UserLocalFileEntryDisplay[] {
  return userMessageLocalFileEntries(message.content);
}

function consoleEntryClass(entry: UserConsoleEntryDisplay) {
  return `level-${entry.level.toLowerCase()}`;
}

function markdownInlineCode(value: string) {
  const backtickRuns = value.match(/`+/g) ?? [];
  const fence = "`".repeat(Math.max(0, ...backtickRuns.map((run) => run.length)) + 1);
  const padding = value.startsWith("`") || value.endsWith("`") ? " " : "";
  return `${fence}${padding}${value}${padding}${fence}`;
}

function localFileEntryMarkdown(entry: UserLocalFileEntryDisplay) {
  return markdownInlineCode(entry.path);
}

function localFileEntryKey(entry: UserLocalFileEntryDisplay, index: number) {
  return `${entry.kind}:${entry.path}:${index}`;
}

function isCompactHandoffMessage(msg: ChatMessage) {
  return msg.role === "assistant" && msg.content.startsWith("## Context Handoff");
}

function hasKnowledgeMutationToolCall(msg: ChatMessage) {
  return !!msg.toolCalls?.some((toolCall) =>
    ["knowledge_create", "knowledge_edit", "knowledge_move", "knowledge_delete"].includes(toolCall.name),
  );
}

const toolOutputMap = computed<Record<string, string>>(() => {
  const map: Record<string, string> = {};
  for (const msg of props.messages) {
    if (msg.role === "tool" && msg.toolCallId) {
      map[msg.toolCallId] = msg.content;
    }
  }
  return map;
});

const toolOutputImageMap = computed<Record<string, ImageAttachment[]>>(() => {
  const map: Record<string, ImageAttachment[]> = {};
  for (const msg of props.messages) {
    if (msg.role === "tool" && msg.toolCallId && msg.images && msg.images.length > 0) {
      map[msg.toolCallId] = msg.images;
    }
  }
  return map;
});

const visibleMessages = computed(() =>
  props.messages.filter((msg) => {
    const status = msg.knowledgeProposal?.status;
    return status !== "stale" && status !== "invalidated";
  }),
);

const toolCallHandoff = ref<ToolCallHandoffState | null>(null);
const toolCallHandoffQuiet = ref(false);
const retainedCollapsedToolCallMatchState = ref<ToolCallMatchState>(emptyToolCallMatchState());
let toolCallHandoffSequence = 0;
let toolCallHandoffArmTimer: ReturnType<typeof setTimeout> | null = null;
let toolCallHandoffFrameA = 0;
let toolCallHandoffFrameB = 0;

function requestToolHandoffFrame(callback: () => void): number {
  if (typeof requestAnimationFrame === "function") {
    return requestAnimationFrame(() => callback());
  }
  if (typeof window !== "undefined") {
    return window.setTimeout(callback, 16);
  }
  return globalThis.setTimeout(callback, 16) as unknown as number;
}

function cancelToolHandoffFrame(handle: number) {
  if (typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(handle);
    return;
  }
  clearTimeout(handle);
}

function setToolCallHandoffQuiet(quiet: boolean) {
  if (toolCallHandoffQuiet.value === quiet) return;
  toolCallHandoffQuiet.value = quiet;
  emit("toolHandoffQuietChange", quiet);
}

function clearToolCallHandoffTimers() {
  if (toolCallHandoffArmTimer) {
    clearTimeout(toolCallHandoffArmTimer);
    toolCallHandoffArmTimer = null;
  }
  if (toolCallHandoffFrameA) {
    cancelToolHandoffFrame(toolCallHandoffFrameA);
    toolCallHandoffFrameA = 0;
  }
  if (toolCallHandoffFrameB) {
    cancelToolHandoffFrame(toolCallHandoffFrameB);
    toolCallHandoffFrameB = 0;
  }
}

function cloneToolCallDisplay(toolCall: ToolCallDisplay): ToolCallDisplay {
  const clone: ToolCallDisplay = {
    id: toolCall.id,
    name: toolCall.name,
    arguments: toolCall.arguments,
    status: toolCall.status,
    order: toolCall.order,
    output: toolCall.output,
    nestedToolCalls: toolCall.nestedToolCalls?.map((nestedToolCall) => cloneToolCallDisplay(nestedToolCall)),
  };
  if (toolCall.images && toolCall.images.length > 0) {
    clone.images = [...toolCall.images];
  }
  return clone;
}

function cloneToolCallDisplays(toolCalls: ToolCallDisplay[]) {
  return toolCalls.map((toolCall) => cloneToolCallDisplay(toolCall));
}

function traceToolCollapse(event: string, detail?: Record<string, unknown>) {
  logToolCollapseTrace(`transcript:${props.variant}`, event, detail);
}

function toolLayoutToolCallIds(toolCalls: ToolCallDisplay[]) {
  return toolCalls.map((toolCall) => toolCall.id).join(",");
}

function toolLayoutStatuses(toolCalls: ToolCallDisplay[]) {
  return toolCalls.map((toolCall) => `${toolCall.id}:${toolCall.status}`).join(",");
}

function messageListLayoutSnapshot(messages: ChatMessage[] | undefined) {
  const list = messages ?? [];
  const lastMessage = list[list.length - 1] ?? null;
  const lastAssistantMessage = [...list].reverse().find((message) => message.role === "assistant") ?? null;
  const textPartLengths = lastAssistantMessage?.renderParts
    ?.filter((part): part is Extract<AssistantRenderPart, { kind: "text" }> => part.kind === "text")
    .map((part) => part.content.length) ?? [];

  return {
    count: list.length,
    lastRole: lastMessage?.role ?? "",
    lastId: lastMessage?.id ?? "",
    lastContentLen: lastMessage?.content.length ?? 0,
    lastRenderPartKinds: lastMessage?.renderParts?.map((part) => part.kind) ?? [],
    lastAssistantId: lastAssistantMessage?.id ?? "",
    lastAssistantContentLen: lastAssistantMessage?.content.length ?? 0,
    lastAssistantRenderPartKinds: lastAssistantMessage?.renderParts?.map((part) => part.kind) ?? [],
    lastAssistantTextPartLengths: textPartLengths,
  };
}

function messageOrderTraceSnapshot(messages: ChatMessage[] | undefined) {
  return (messages ?? []).map((message, index) => ({
    index,
    id: message.id,
    role: message.role,
    contentLen: message.content.length,
    contentPreview: previewTraceText(message.content, 48),
    toolCallId: message.toolCallId ?? null,
    toolCallIds: message.toolCalls?.map((toolCall) => toolCall.id) ?? [],
    renderPartKinds: message.renderParts?.map((part) => part.kind) ?? [],
  }));
}

function parseTraceJson(value: string | undefined) {
  if (!value) return null;
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return value;
  }
}

function traceToolLayoutChange(reason: string, detail: Record<string, unknown> = {}) {
  traceToolBlockLayoutChange({
    scope: `chat-transcript:${props.variant}`,
    reason,
    scrollElement: scrollRef.value,
    contentElement: contentRef.value,
    detail: {
      sessionKey: props.sessionKey ?? "",
      isStreaming: props.isStreaming,
      activeToolCallCount: props.activeToolCalls.length,
      activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
      hasHandoff: !!toolCallHandoff.value,
      handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
      handoffCollapseFinished: toolCallHandoff.value?.collapseFinished ?? false,
      ...detail,
    },
  });
}

function shouldRetainCollapsedToolCallHandoff(handoff: ToolCallHandoffState) {
  return handoff.collapseFinished || (handoff.collapseArmed && handoff.willAutoCollapse);
}

function retainCollapsedToolCallHandoff(handoff: ToolCallHandoffState, reason: string) {
  const retainedToolCalls = handoff.collapseCandidateToolCalls.length > 0
    ? handoff.collapseCandidateToolCalls
    : handoff.toolCalls;
  if (!shouldRetainCollapsedToolCallHandoff(handoff)) {
    traceToolCollapse("retainCollapsedToolCallHandoffSkipped", {
      reason,
      renderKey: handoff.renderKey,
      toolCallCount: retainedToolCalls.length,
      toolCallIds: Array.from(handoff.toolCallIds),
      collapseArmed: handoff.collapseArmed,
      collapseFinished: handoff.collapseFinished,
      willAutoCollapse: handoff.willAutoCollapse,
      retainedEntryCount: toolCallMatchEntryCount(retainedCollapsedToolCallMatchState.value),
    });
    return;
  }
  retainedCollapsedToolCallMatchState.value = mergeToolCallMatchStates(
    retainedCollapsedToolCallMatchState.value,
    collectToolCallDisplayMatchState(retainedToolCalls),
  );
  traceToolCollapse("retainCollapsedToolCallHandoff", {
    reason,
    renderKey: handoff.renderKey,
    toolCallCount: retainedToolCalls.length,
    retainedToolCallIds: retainedToolCalls.map((toolCall) => toolCall.id),
    collapseArmed: handoff.collapseArmed,
    collapseFinished: handoff.collapseFinished,
    willAutoCollapse: handoff.willAutoCollapse,
    retainedEntryCount: toolCallMatchEntryCount(retainedCollapsedToolCallMatchState.value),
  });
}

function clearRetainedCollapsedToolCalls(reason: string) {
  if (!hasToolCallMatchState(retainedCollapsedToolCallMatchState.value)) return;
  traceToolCollapse("clearRetainedCollapsedToolCalls", {
    reason,
    retainedEntryCount: toolCallMatchEntryCount(retainedCollapsedToolCallMatchState.value),
  });
  retainedCollapsedToolCallMatchState.value = emptyToolCallMatchState();
}

function clearToolCallHandoff(reason = "clear") {
  const handoff = toolCallHandoff.value;
  if (handoff) {
    traceToolLayoutChange("clearToolCallHandoff", {
      reason,
      renderKey: handoff.renderKey,
      toolCallCount: handoff.toolCalls.length,
      toolCallIds: Array.from(handoff.toolCallIds),
      collapseArmed: handoff.collapseArmed,
      collapseFinished: handoff.collapseFinished,
    });
    traceToolCollapse("clearToolCallHandoff", {
      reason,
      renderKey: handoff.renderKey,
      toolCallCount: handoff.toolCalls.length,
      collapseArmed: handoff.collapseArmed,
      collapseFinished: handoff.collapseFinished,
    });
  }
  clearToolCallHandoffTimers();
  toolCallHandoff.value = null;
  setToolCallHandoffQuiet(false);
}

watch(
  () => props.sessionKey,
  (sessionKey, previousSessionKey) => {
    if (sessionKey === previousSessionKey) return;
    clearToolCallHandoff("session-key-changed");
    clearRetainedCollapsedToolCalls("session-key-changed");
  },
  { flush: "sync" },
);

const hasVisibleStreamingText = computed(() => props.streamingText.trim().length > 0);
const hasVisibleStreamingTextAfterToolHandoff = computed(() => {
  const handoff = toolCallHandoff.value;
  if (!handoff) return hasVisibleStreamingText.value;
  if (hasVisibleTextPartAfterToolCalls(props.liveRenderParts, handoff.toolCalls)) {
    return true;
  }
  const handoffLastToolOrder = lastToolCallRenderOrder(handoff.toolCalls);
  return handoffLastToolOrder > 0
    && hasVisibleStreamingText.value
    && props.streamingTextOrder > handoffLastToolOrder;
});
const shouldArmToolCallHandoffCollapse = computed(
  () => hasVisibleStreamingTextAfterToolHandoff.value || !props.isStreaming,
);

function scheduleToolCallHandoffCollapse() {
  const handoff = toolCallHandoff.value;
  if (!handoff || handoff.collapseArmed) return;

  traceToolCollapse("scheduleToolCallHandoffCollapse", {
    renderKey: handoff.renderKey,
    createdAt: handoff.createdAt,
    toolCallCount: handoff.toolCalls.length,
    visibleStreamingText: hasVisibleStreamingText.value,
    visibleStreamingTextAfterHandoff: hasVisibleStreamingTextAfterToolHandoff.value,
    streamingTextLen: props.streamingText.length,
    isStreaming: props.isStreaming,
  });

  clearToolCallHandoffTimers();
  const targetRenderKey = handoff.renderKey;

  nextTick(() => {
    toolCallHandoffFrameA = requestToolHandoffFrame(() => {
      toolCallHandoffFrameA = 0;
      toolCallHandoffFrameB = requestToolHandoffFrame(() => {
        toolCallHandoffFrameB = 0;
        const current = toolCallHandoff.value;
        if (!current || current.renderKey !== targetRenderKey) return;

        const remainingDelay = Math.max(
          TOOL_HANDOFF_MIN_VISIBLE_MS - (Date.now() - current.createdAt),
          0,
        );
        traceToolCollapse("scheduleToolCallHandoffCollapse:armed-delay", {
          renderKey: current.renderKey,
          remainingDelay,
          ageMs: Date.now() - current.createdAt,
        });
        toolCallHandoffArmTimer = setTimeout(() => {
          toolCallHandoffArmTimer = null;
          const nextHandoff = toolCallHandoff.value;
          if (!nextHandoff || nextHandoff.renderKey !== targetRenderKey) return;

          if (!transientToolCallsCanCollapse.value) {
            traceToolCollapse("collapseSkipped", {
              renderKey: nextHandoff.renderKey,
              toolCallCount: transientCollapseCandidateToolCalls.value.length,
              promotableHistoryToolCallCount: promotableHistoryToolCalls.value.toolCalls.length,
            });
            clearToolCallHandoff("not-auto-collapsible");
            return;
          }

          traceToolLayoutChange("toolCallHandoffCollapseArmed", {
            renderKey: nextHandoff.renderKey,
            toolCallIds: Array.from(nextHandoff.toolCallIds),
            collapseCandidateToolCallCount: transientCollapseCandidateToolCalls.value.length,
            promotableHistoryToolCallCount: promotableHistoryToolCalls.value.toolCalls.length,
          });
          setToolCallHandoffQuiet(true);
          traceToolCollapse("collapseArmed", {
            renderKey: nextHandoff.renderKey,
            toolCallIds: Array.from(nextHandoff.toolCallIds),
            collapseCandidateToolCallCount: transientCollapseCandidateToolCalls.value.length,
            promotableHistoryToolCallCount: promotableHistoryToolCalls.value.toolCalls.length,
            streamingTextLen: props.streamingText.length,
            isStreaming: props.isStreaming,
          });
          toolCallHandoff.value = {
            ...nextHandoff,
            collapseCandidateToolCalls: cloneToolCallDisplays(transientCollapseCandidateToolCalls.value),
            collapseArmed: true,
            collapseFinished: false,
          };
        }, remainingDelay);
      });
    });
  });
}

function beginToolCallHandoff(previousToolCalls: ToolCallDisplay[]) {
  const toolCalls = previousToolCalls.map((toolCall) => cloneToolCallDisplay(toolCall));
  if (toolCalls.length === 0) {
    clearToolCallHandoff("begin-empty");
    return;
  }

  traceToolLayoutChange("beginToolCallHandoff", {
    toolCallCount: toolCalls.length,
    toolCallIds: toolCalls.map((toolCall) => toolCall.id),
    statuses: toolLayoutStatuses(toolCalls),
  });
  clearToolCallHandoffTimers();
  setToolCallHandoffQuiet(false);
  toolCallHandoff.value = {
    renderKey: `tool-handoff-${++toolCallHandoffSequence}:${toolCalls.map((toolCall) => toolCall.id).join(",")}`,
    createdAt: Date.now(),
    toolCalls,
    toolCallIds: collectToolCallDisplayIds(toolCalls),
    toolCallMatchState: collectToolCallDisplayMatchState(toolCalls),
    collapseCandidateToolCalls: cloneToolCallDisplays(toolCalls),
    willAutoCollapse: summarizeToolCallBatch(toolCalls, displaySettings.compactToolCalls).canCollapse,
    collapseArmed: false,
    collapseFinished: false,
  };

  traceToolCollapse("beginToolCallHandoff", {
    renderKey: toolCallHandoff.value.renderKey,
    toolCallCount: toolCalls.length,
    toolCallIds: Array.from(toolCallHandoff.value.toolCallIds),
    willAutoCollapse: toolCallHandoff.value.willAutoCollapse,
    streamingTextLen: props.streamingText.length,
    isStreaming: props.isStreaming,
  });

  if (shouldArmToolCallHandoffCollapse.value) {
    scheduleToolCallHandoffCollapse();
  }
}

function toolCallTreeHasAnyIds(
  toolCalls: ToolCallInfo[] | undefined,
  targetMatchState: ToolCallMatchState,
): boolean {
  if (
    !toolCalls
    || toolCalls.length === 0
    || (targetMatchState.ids.size === 0 && targetMatchState.fingerprintCounts.size === 0)
  ) {
    return false;
  }
  return toolCalls.some((toolCall) =>
    targetMatchState.ids.has(toolCall.id)
    || targetMatchState.fingerprintCounts.has(getToolCallInfoFingerprint(toolCall))
    || toolCallTreeHasAnyIds(toolCall.nestedToolCalls, targetMatchState),
  );
}

function hasVisibleMessagePayload(message: ChatMessage) {
  const status = message.knowledgeProposal?.status;
  if (status === "stale" || status === "invalidated") return false;
  if (message.role === "tool") return false;
  return !!(
    message.content.trim()
    || message.thinkingContent?.trim()
    || message.knowledgeProposal
    || message.images?.length
    || message.assetRefs?.length
    || (message.role === "user" && messageConsoleEntries(message).length > 0)
    || (message.role === "user" && messageLocalFileEntries(message).length > 0)
    || (message.role === "user" && userMessageDisplayContent(message).trim())
  );
}

function shouldReleaseToolCallHandoffToHistory(
  messages: ChatMessage[],
  targetMatchState: ToolCallMatchState,
) {
  let matchedToolMessage = false;
  for (const message of messages) {
    const matchesTool = toolCallTreeHasAnyIds(message.toolCalls, targetMatchState);
    if (matchesTool && hasVisibleMessagePayload(message)) {
      return true;
    }
    if (matchedToolMessage && hasVisibleMessagePayload(message)) {
      return true;
    }
    if (matchesTool) {
      matchedToolMessage = true;
    }
  }
  return false;
}

function hasVisibleUserMessageAfterToolCallMatch(
  messages: ChatMessage[],
  targetMatchState: ToolCallMatchState,
) {
  let matchedToolMessage = false;
  for (const message of messages) {
    if (matchedToolMessage && message.role === "user" && hasVisibleMessagePayload(message)) {
      return true;
    }
    if (toolCallTreeHasAnyIds(message.toolCalls, targetMatchState)) {
      matchedToolMessage = true;
    }
  }
  return false;
}

watch(
  () => props.activeToolCalls,
  (activeToolCalls, previousToolCalls) => {
    if (activeToolCalls !== previousToolCalls) {
      traceToolLayoutChange("activeToolCallsChanged", {
        previousCount: previousToolCalls?.length ?? 0,
        previousIds: previousToolCalls?.map((toolCall) => toolCall.id) ?? [],
        nextCount: activeToolCalls.length,
        nextIds: activeToolCalls.map((toolCall) => toolCall.id),
        nextStatuses: toolLayoutStatuses(activeToolCalls),
      });
    }
    if (activeToolCalls.length === 0 && previousToolCalls && previousToolCalls.length > 0) {
      const previousMatchState = collectToolCallDisplayMatchState(previousToolCalls);
      traceToolCollapse("activeToolCallsCleared", {
        previousCount: previousToolCalls.length,
        previousIds: previousToolCalls.map((toolCall) => toolCall.id),
        streamingTextLen: props.streamingText.length,
        isStreaming: props.isStreaming,
      });
      if (hasVisibleUserMessageAfterToolCallMatch(props.messages, previousMatchState)) {
        clearToolCallHandoff("active-cleared-history-before-inserted-user");
        return;
      }
      if (!props.isStreaming && shouldReleaseToolCallHandoffToHistory(props.messages, previousMatchState)) {
        clearToolCallHandoff("active-cleared-history-ready");
        return;
      }
      beginToolCallHandoff(previousToolCalls);
    }
  },
  { flush: "sync" },
);

watch(
  () => props.activeToolCalls.length,
  (activeToolCallCount, previousActiveToolCallCount) => {
    if (activeToolCallCount > 0 && toolCallHandoff.value) {
      traceToolLayoutChange("activeToolCallsResumedWithHandoff", {
        previousActiveToolCallCount,
        activeToolCallCount,
        handoffRenderKey: toolCallHandoff.value.renderKey,
        handoffToolCallIds: Array.from(toolCallHandoff.value.toolCallIds),
      });
      traceToolCollapse("activeToolCallsResumedWithHandoff", {
        previousActiveToolCallCount,
        activeToolCallCount,
        activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
        handoffRenderKey: toolCallHandoff.value.renderKey,
        handoffToolCallIds: Array.from(toolCallHandoff.value.toolCallIds),
        handoffCollapseArmed: toolCallHandoff.value.collapseArmed,
        handoffCollapseFinished: toolCallHandoff.value.collapseFinished,
        isStreaming: props.isStreaming,
      });
      retainCollapsedToolCallHandoff(toolCallHandoff.value, "active-tool-calls-resumed");
      clearToolCallHandoff("active-tool-calls-resumed");
    }
  },
  { flush: "sync" },
);

let lastShouldArmToolCallHandoffCollapse = shouldArmToolCallHandoffCollapse.value;

watch(shouldArmToolCallHandoffCollapse, (shouldArm) => {
  traceToolCollapse("shouldArmToolCallHandoffCollapseChanged", {
    previous: lastShouldArmToolCallHandoffCollapse,
    next: shouldArm,
    hasVisibleStreamingText: hasVisibleStreamingText.value,
    hasVisibleStreamingTextAfterToolHandoff: hasVisibleStreamingTextAfterToolHandoff.value,
    streamingTextLen: props.streamingText.length,
    isStreaming: props.isStreaming,
    hasHandoff: !!toolCallHandoff.value,
  });
  lastShouldArmToolCallHandoffCollapse = shouldArm;
  if (shouldArm && toolCallHandoff.value) {
    scheduleToolCallHandoffCollapse();
  }
});

watch(
  () => props.messages,
  (messages, previous) => {
    if (messages === previous) return;
    const hasHandoff = !!toolCallHandoff.value;
    traceToolLayoutChange(hasHandoff ? "messagesChangedDuringToolHandoff" : "messagesChanged", {
      previousCount: previous?.length ?? 0,
      nextCount: messages.length,
      previousMessages: messageListLayoutSnapshot(previous),
      nextMessages: messageListLayoutSnapshot(messages),
      streamingTextLen: props.streamingText.length,
      isStreaming: props.isStreaming,
      hasHandoff,
      handoffRenderKey: toolCallHandoff.value?.renderKey ?? "",
      handoffToolCallIds: toolCallHandoff.value ? Array.from(toolCallHandoff.value.toolCallIds) : [],
    });
    traceToolCollapse("messagesOrderChanged", {
      previous: messageOrderTraceSnapshot(previous),
      next: messageOrderTraceSnapshot(messages),
      isStreaming: props.isStreaming,
      activeToolCallCount: props.activeToolCalls.length,
      activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
      hasHandoff,
      handoffRenderKey: toolCallHandoff.value?.renderKey ?? "",
      handoffToolCallIds: toolCallHandoff.value ? Array.from(toolCallHandoff.value.toolCallIds) : [],
    });
    if (!toolCallHandoff.value) return;
    if (hasVisibleUserMessageAfterToolCallMatch(messages, toolCallHandoff.value.toolCallMatchState)) {
      clearToolCallHandoff("handoff-history-before-inserted-user");
      return;
    }
    if (!props.isStreaming && shouldReleaseToolCallHandoffToHistory(messages, toolCallHandoff.value.toolCallMatchState)) {
      clearToolCallHandoff("handoff-followed-by-history-message");
      return;
    }
    if (messages.some((message) => toolCallTreeHasAnyIds(message.toolCalls, toolCallHandoff.value!.toolCallMatchState))) {
      return;
    }
    clearToolCallHandoff("handoff-ids-missing-from-messages");
  },
  { flush: "sync" },
);

watch(hasVisibleStreamingText, (visible, previousVisible) => {
  traceToolLayoutChange("visibleStreamingTextChanged", {
    previous: previousVisible,
    next: visible,
    streamingTextLen: props.streamingText.length,
  });
  traceToolCollapse("hasVisibleStreamingTextChanged", {
    previous: previousVisible,
    next: visible,
    streamingTextLen: props.streamingText.length,
    streamingTextPreview: props.streamingText ? previewTraceText(props.streamingText, 64) : "",
    isStreaming: props.isStreaming,
  });
});

watch(
  () => props.isStreaming,
  (isStreaming, previousStreaming) => {
    const handoff = toolCallHandoff.value;
    traceToolLayoutChange("isStreamingChanged", {
      previous: previousStreaming,
      next: isStreaming,
      hasHandoff: !!handoff,
      collapseArmed: handoff?.collapseArmed ?? false,
      collapseFinished: handoff?.collapseFinished ?? false,
    });
    traceToolCollapse("isStreamingChanged", {
      previous: previousStreaming,
      next: isStreaming,
      hasHandoff: !!handoff,
      collapseArmed: handoff?.collapseArmed ?? false,
      collapseFinished: handoff?.collapseFinished ?? false,
    });
    if (!isStreaming) {
      clearRetainedCollapsedToolCalls("stream-ended");
    }
    if (isStreaming || !handoff?.collapseFinished) return;
    clearToolCallHandoff("stream-ended-after-collapse");
  },
  { flush: "sync" },
);

onUnmounted(() => {
  clearToolCallHandoff("unmount");
});

const activeToolCallMatchState = computed<ToolCallMatchState>(() => {
  if (props.activeToolCalls.length > 0) {
    return collectToolCallDisplayIdMatchState(props.activeToolCalls);
  }
  return toolCallHandoff.value?.toolCallMatchState ?? {
    ids: new Set<string>(),
    fingerprintCounts: new Map<string, number>(),
    idFingerprints: new Map<string, string>(),
  };
});

function toolCallInfosForMessage(message: Pick<ChatMessage, "toolCalls" | "renderParts">) {
  if (message.toolCalls) return message.toolCalls;
  const toolParts = message.renderParts
    ?.filter((part): part is Extract<AssistantRenderPart, { kind: "toolCall" }> => part.kind === "toolCall")
    .map((part) => part.toolCall);
  return toolParts && toolParts.length > 0 ? toolParts : undefined;
}

function hasExplicitDisplayToolCalls(item: Pick<MessageRenderItem, "displayToolCalls">) {
  return Object.prototype.hasOwnProperty.call(item, "displayToolCalls");
}

function toolCallsFromInfos(toolCalls: ToolCallInfo[] | undefined) {
  return buildMessageToolCalls({ toolCalls }, toolOutputMap.value, toolOutputImageMap.value);
}

function shouldHideThinkingBlocks() {
  return displaySettings.hideThinkingBlocks !== false;
}

function shouldRenderHistoryThinkingBlock(item: Pick<MessageRenderItem, "message">) {
  return !shouldHideThinkingBlocks() && !!item.message.thinkingContent?.trim();
}

function shouldRenderTransientThinkingSegment(part: Extract<AssistantRenderPart, { kind: "thinking" }>) {
  return !!part.active || (!shouldHideThinkingBlocks() && part.content.trim().length > 0);
}

function shouldRenderItem(item: MessageRenderItem) {
  if (item.message.knowledgeProposal) return true;

  if (item.message.role === "user") {
    return !!(
      userMessageDisplayContent(item.message)
      || messageLocalFileEntries(item.message).length > 0
      || (props.showUserImages && item.message.images && item.message.images.length > 0)
      || (props.enableIntentBadges && messageIntentBadges(item.message).length > 0)
    );
  }

  return !!(
    renderPartsForMessage(item).length > 0
    || item.attachedKnowledgeProposals.length > 0
  );
}

function hasToolCallMatchState(state: ToolCallMatchState) {
  return state.ids.size > 0 || state.fingerprintCounts.size > 0;
}

function emptyToolCallMatchState(): ToolCallMatchState {
  return {
    ids: new Set<string>(),
    fingerprintCounts: new Map<string, number>(),
    idFingerprints: new Map<string, string>(),
  };
}

function toolCallMatchEntryCount(state: ToolCallMatchState) {
  let fingerprintCount = 0;
  for (const count of state.fingerprintCounts.values()) {
    fingerprintCount += count;
  }
  return state.ids.size + fingerprintCount;
}

function buildTailHiddenToolCallMap(
  groups: MessageGroup[],
  hiddenToolCallMatchState: ToolCallMatchState,
) {
  const hiddenToolCallsByItemId = new Map<string, ToolCallInfo[] | undefined>();
  if (!hasToolCallMatchState(hiddenToolCallMatchState)) return hiddenToolCallsByItemId;

  const tailGroup = groups[groups.length - 1];
  if (!tailGroup || tailGroup.role !== "assistant") return hiddenToolCallsByItemId;

  const consumableMatchState = cloneToolCallMatchState(hiddenToolCallMatchState);
  for (let index = tailGroup.items.length - 1; index >= 0; index -= 1) {
    if (!hasToolCallMatchState(consumableMatchState)) break;
    const item = tailGroup.items[index];
    const itemToolCalls = item ? toolCallInfosForMessage(item.message) : undefined;
    if (
      !item
      || item.hidden
      || item.message.knowledgeProposal
      || item.message.role !== "assistant"
      || !itemToolCalls
      || itemToolCalls.length === 0
    ) {
      continue;
    }

    const previousMatchEntryCount = toolCallMatchEntryCount(consumableMatchState);
    const toolCalls = filterToolCallsByConsumableMatchState(
      itemToolCalls,
      consumableMatchState,
    );
    if (toolCallMatchEntryCount(consumableMatchState) !== previousMatchEntryCount) {
      hiddenToolCallsByItemId.set(item.id, toolCalls ?? []);
    }
  }

  return hiddenToolCallsByItemId;
}

function buildGroupedMessages(hiddenToolCallMatchState: ToolCallMatchState): MessageGroup[] {
  const groups: MessageGroup[] = [];
  const flatItems: MessageRenderItem[] = [];
  let order = 0;
  for (const msg of visibleMessages.value) {
    if (msg.role === "tool") continue;
    const renderItem: MessageRenderItem = {
      id: msg.id,
      order,
      message: msg,
      attachedKnowledgeProposals: [],
      hidden: false,
    };
    order += 1;
    flatItems.push(renderItem);
    const last = groups[groups.length - 1];
    const isHandoff = isCompactHandoffMessage(msg);
    const lastIsHandoff = !!last?.items.some((item) => isCompactHandoffMessage(item.message));
    if (last && last.role === msg.role && !isHandoff && !lastIsHandoff) {
      last.items.push(renderItem);
    } else {
      groups.push({ id: msg.id, role: msg.role as "user" | "assistant", items: [renderItem] });
    }
  }

  for (const group of groups) {
    if (group.role !== "assistant") continue;
    for (let index = 0; index < group.items.length; index += 1) {
      const item = group.items[index];
      if (!item?.message.knowledgeProposal) continue;
      const nextRequestTool = group.items.find((candidate, candidateIndex) =>
        candidateIndex > index
        && !candidate.message.knowledgeProposal
        && hasKnowledgeMutationToolCall(candidate.message),
      );
      const prevRequestTool = [...group.items].reverse().find((candidate) =>
        candidate.order < item.order
        && !candidate.message.knowledgeProposal
        && hasKnowledgeMutationToolCall(candidate.message),
      );
      const target = nextRequestTool ?? prevRequestTool;
      if (!target) continue;
      target.attachedKnowledgeProposals.push(item.message);
      item.hidden = true;
    }
  }

  const hiddenToolCallsByItemId = buildTailHiddenToolCallMap(groups, hiddenToolCallMatchState);

  return groups
    .map((group) => ({
      ...group,
      items:
        group.role === "assistant"
          ? mergeSequentialAssistantToolCalls(
              group.items
                .filter((item) => !item.hidden)
                .map((item) => ({
                  ...item,
                  content: item.message.content,
                  thinkingContent: item.message.thinkingContent,
                  renderParts: item.message.renderParts,
                  toolCalls: hiddenToolCallsByItemId.has(item.id)
                    ? hiddenToolCallsByItemId.get(item.id)
                    : toolCallInfosForMessage(item.message),
                  attachedKnowledgeProposalCount: item.attachedKnowledgeProposals.length,
                  isKnowledgeProposal: !!item.message.knowledgeProposal,
                })),
            ).map(
              ({
                content: _content,
                thinkingContent: _thinkingContent,
                renderParts: _renderParts,
                toolCalls: _toolCalls,
                attachedKnowledgeProposalCount: _proposalCount,
                isKnowledgeProposal: _isKnowledgeProposal,
                ...item
              }) => item,
            )
          : group.items.filter((item) => !item.hidden),
    }))
    .map((group) => ({
      ...group,
      items: group.items.filter((item) => shouldRenderItem(item)),
    }))
    .filter((group) => group.items.length > 0);
}

const baseGroupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(activeToolCallMatchState.value));

const shouldPromoteHistoryToolCalls = computed(
  () => !!toolCallHandoff.value || props.activeToolCalls.length > 0,
);

function emptyPromotedHistoryToolCalls(): PromotedHistoryToolCallsState {
  return {
    itemIds: new Set<string>(),
    toolCallIds: new Set<string>(),
    toolCallMatchState: {
      ids: new Set<string>(),
      fingerprintCounts: new Map<string, number>(),
      idFingerprints: new Map<string, string>(),
    },
    toolCalls: [],
  };
}

const promotableHistoryToolCalls = computed<PromotedHistoryToolCallsState>(() => {
  if (!shouldPromoteHistoryToolCalls.value) return emptyPromotedHistoryToolCalls();

  const lastGroup = baseGroupedMessages.value[baseGroupedMessages.value.length - 1];
  if (!lastGroup || lastGroup.role !== "assistant") {
    return emptyPromotedHistoryToolCalls();
  }

  const collectedItemIds = new Set<string>();
  const collectedBatches: ToolCallDisplay[][] = [];
  const segments = historyRenderSegmentsForGroup(lastGroup);

  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index];
    if (!segment || segment.type !== "toolCalls") break;
    if (segment.toolCalls.length === 0) break;
    for (const itemId of segment.itemIds) {
      collectedItemIds.add(itemId);
    }
    collectedBatches.unshift(cloneToolCallDisplays(segment.toolCalls));
  }

  if (collectedBatches.length === 0) return emptyPromotedHistoryToolCalls();

  const toolCalls = collectedBatches.flat();
  return {
    itemIds: collectedItemIds,
    toolCallIds: collectToolCallDisplayIds(toolCalls),
    toolCallMatchState: collectToolCallDisplayMatchState(toolCalls),
    toolCalls,
  };
});

const transientOwnedToolCalls = computed(() =>
  props.activeToolCalls.length > 0
    ? props.activeToolCalls
    : (toolCallHandoff.value?.toolCalls ?? []),
);

const transientCollapseCandidateToolCalls = computed(() => {
  if (promotableHistoryToolCalls.value.toolCalls.length === 0) {
    return transientOwnedToolCalls.value;
  }
  return mergeToolCallDisplaysWithoutDuplicates(
    promotableHistoryToolCalls.value.toolCalls,
    transientOwnedToolCalls.value,
  );
});

const transientToolCallsCanCollapse = computed(() =>
  summarizeToolCallBatch(transientCollapseCandidateToolCalls.value, displaySettings.compactToolCalls).canCollapse,
);

const shouldKeepPromotedHistoryToolCallsInTransient = computed(() =>
  props.activeToolCalls.length > 0
  || (!!toolCallHandoff.value && transientToolCallsCanCollapse.value),
);

const shouldHidePromotedHistoryToolCalls = computed(() =>
  promotableHistoryToolCalls.value.toolCalls.length > 0
  && shouldKeepPromotedHistoryToolCallsInTransient.value,
);

watch(shouldHidePromotedHistoryToolCalls, (next, previous) => {
  traceToolLayoutChange("promotedHistoryToolCallsVisibilityChanged", {
    previous,
    next,
    promotedToolCallCount: promotableHistoryToolCalls.value.toolCalls.length,
    promotedToolCallIds: promotableHistoryToolCalls.value.toolCalls.map((toolCall) => toolCall.id),
    keepPromotedInTransient: shouldKeepPromotedHistoryToolCallsInTransient.value,
  });
  traceToolCollapse("promotedHistoryToolCallsVisibilityChanged", {
    previous,
    next,
    promotedToolCallCount: promotableHistoryToolCalls.value.toolCalls.length,
    promotedToolCallIds: promotableHistoryToolCalls.value.toolCalls.map((toolCall) => toolCall.id),
    activeToolCallCount: props.activeToolCalls.length,
    activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
    hasHandoff: !!toolCallHandoff.value,
    handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
    keepPromotedInTransient: shouldKeepPromotedHistoryToolCallsInTransient.value,
    isStreaming: props.isStreaming,
  });
});

// Fail-open: only hide promoted history tool calls that the transient region
// actually renders this tick. Anything transient drops stays in history, so the
// trailing tool batch (the "last message") can never vanish from both regions.
// `transientRenderedToolCalls` is defined alongside `transientRenderSegments`
// below; the closure resolves it lazily so there is no initialization cycle
// (transient rendering never reads the hidden state).
const promotedToolVisibility = computed(() => {
  if (!shouldHidePromotedHistoryToolCalls.value) return null;
  return planPromotedToolVisibility({
    promotedToolCalls: promotableHistoryToolCalls.value.toolCalls,
    transientRenderedToolCalls: transientRenderedToolCalls.value,
  });
});

const historyHiddenToolCallMatchState = computed<ToolCallMatchState>(() => {
  const visibility = promotedToolVisibility.value;
  if (!visibility) {
    return activeToolCallMatchState.value;
  }
  return mergeToolCallMatchStates(
    activeToolCallMatchState.value,
    visibility.hiddenMatchState,
  );
});

const groupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(historyHiddenToolCallMatchState.value));

const shouldRenderPromotedHistoryToolCallsInTransient = computed(() =>
  promotableHistoryToolCalls.value.toolCalls.length > 0
  && shouldKeepPromotedHistoryToolCallsInTransient.value,
);

function toolCallMatchStateMemoKey(state: ToolCallMatchState) {
  const ids = Array.from(state.ids).sort().join(",");
  const fingerprints = Array.from(state.fingerprintCounts.entries())
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([fingerprint, count]) => `${fingerprint}:${count}`)
    .join(",");
  return `${ids}|${fingerprints}`;
}

const retainedCollapsedToolCallMemoKey = computed(() =>
  toolCallMatchStateMemoKey(retainedCollapsedToolCallMatchState.value));

const historyLiveLayoutMemoKey = computed(() => [
  isStreamingContinuation.value ? "continuation" : "separate",
  hasTransientAssistantMessage.value ? "has-transient" : "no-transient",
  props.isStreaming ? "streaming" : "idle",
  props.isCompacting ? "compacting" : "normal",
  toolCallHandoff.value ? "handoff" : "no-handoff",
  toolCallHandoff.value?.collapseArmed ? "armed" : "unarmed",
  toolCallHandoff.value?.collapseFinished ? "collapsed" : "open",
  shouldHidePromotedHistoryToolCalls.value ? "hide-promoted" : "show-history",
  retainedCollapsedToolCallMemoKey.value,
].join("\u241e"));

const historyStaticRenderMemoKey = computed(() => [
  props.variant,
  props.enableIntentBadges ? "intent" : "no-intent",
  props.showUserImages ? "images" : "no-images",
  props.userContentMode,
  props.userLabel,
  props.assistantLabel,
  props.handoffLabel,
  props.compactedLabel,
  props.thoughtDurationLabel,
  props.thoughtMomentLabel,
  displaySettings.rightAlignUserMessages ? "align-user" : "left-user",
  displaySettings.compactToolCalls ? "compact-tools" : "open-tools",
  displaySettings.hideThinkingBlocks !== false ? "hide-thinking" : "show-thinking",
].join("\u241e"));

function historySelectionMemoKey(group: MessageGroup) {
  const selectedMessageId = props.selectedMessageId;
  if (!selectedMessageId) return "";
  return group.items.some((item) => item.id === selectedMessageId) ? selectedMessageId : "";
}

function historyGroupMemoKey(group: MessageGroup, index: number) {
  const lastGroupIndex = groupedMessages.value.length - 1;
  return [
    historyStaticRenderMemoKey.value,
    historySelectionMemoKey(group),
    index === lastGroupIndex ? historyLiveLayoutMemoKey.value : "",
  ].join("\u241e");
}

const transientToolCalls = computed(() => {
  if (!shouldRenderPromotedHistoryToolCallsInTransient.value) {
    return transientOwnedToolCalls.value;
  }
  return transientCollapseCandidateToolCalls.value;
});

const transientToolCallsAllowCollapse = computed(
  () => !!toolCallHandoff.value && transientToolCallsCanCollapse.value,
);
const transientToolCallsCollapseEnabled = computed(
  () => !!toolCallHandoff.value?.collapseArmed && transientToolCallsCanCollapse.value,
);

watch(promotableHistoryToolCalls, (nextState, previousState) => {
  traceToolLayoutChange("promotableHistoryToolCallsChanged", {
    previousItemCount: previousState?.itemIds.size ?? 0,
    nextItemCount: nextState.itemIds.size,
    nextToolCallCount: nextState.toolCalls.length,
    nextToolCallIds: Array.from(nextState.toolCallIds),
  });
  traceToolCollapse("promotableHistoryToolCallsChanged", {
    previousItemCount: previousState?.itemIds.size ?? 0,
    nextItemCount: nextState.itemIds.size,
    nextToolCallCount: nextState.toolCalls.length,
    nextToolCallIds: Array.from(nextState.toolCallIds),
    hasHandoff: !!toolCallHandoff.value,
  });
});

watch(transientToolCallsCollapseEnabled, (enabled, previousEnabled) => {
  traceToolLayoutChange("transientToolCallsCollapseEnabledChanged", {
    previous: previousEnabled,
    next: enabled,
    transientToolCallCount: transientToolCalls.value.length,
    transientToolCallIds: transientToolCalls.value.map((toolCall) => toolCall.id),
  });
  traceToolCollapse("transientToolCallsCollapseEnabledChanged", {
    previous: previousEnabled,
    next: enabled,
    transientToolCallCount: transientToolCalls.value.length,
    hasHandoff: !!toolCallHandoff.value,
  });
});

function onTransientToolCallsCollapseFinished() {
  if (!toolCallHandoff.value?.collapseArmed) return;
  traceToolLayoutChange("transientToolCallsCollapseFinished", {
    renderKey: toolCallHandoff.value.renderKey,
    toolCallCount: transientToolCalls.value.length,
    toolCallIds: transientToolCalls.value.map((toolCall) => toolCall.id),
  });
  traceToolCollapse("onTransientToolCallsCollapseFinished", {
    renderKey: toolCallHandoff.value.renderKey,
    toolCallCount: transientToolCalls.value.length,
    isStreaming: props.isStreaming,
  });
  setToolCallHandoffQuiet(false);

  if (!toolCallHandoff.value.collapseFinished) {
    toolCallHandoff.value = {
      ...toolCallHandoff.value,
      collapseFinished: true,
    };
  }

  if (!props.isStreaming) {
    clearToolCallHandoff("collapse-finished-after-stream-end");
  }
}

// Rendering reads the throttled copy so per-delta content growth repaints at
// the shared streaming cadence; handoff timing logic stays on the raw prop.
const displayedLiveRenderParts = useThrottledLiveRenderParts(() => props.liveRenderParts);

const canonicalLiveRenderParts = computed(() => {
  if (displayedLiveRenderParts.value.length > 0) {
    assertCanonicalRenderParts(displayedLiveRenderParts.value, "live");
    return [...displayedLiveRenderParts.value].sort(compareAssistantRenderParts);
  }

  const legacyToolCalls: ToolCallInfo[] = props.activeToolCalls.map((toolCall) => ({
    id: toolCall.id,
    name: toolCall.name,
    arguments: toolCall.arguments,
    order: toolCall.order,
    outcome: toolCall.status === "running" ? undefined : toolCall.status,
    recordedOutput: toolCall.output,
    nestedToolCalls: toolCall.nestedToolCalls?.map((nestedToolCall) => ({
      id: nestedToolCall.id,
      name: nestedToolCall.name,
      arguments: nestedToolCall.arguments,
      order: nestedToolCall.order,
      outcome: nestedToolCall.status === "running" ? undefined : nestedToolCall.status,
      recordedOutput: nestedToolCall.output,
    })),
  }));
  return synthesizeLegacyRenderParts({
    id: "__transient__",
    role: "assistant",
    content: props.streamingText,
    createdAt: Date.now() / 1000,
    contentOrder: props.streamingTextOrder,
    thinkingOrder: props.thinkingOrder,
    thinkingContent: props.thinkingText || undefined,
    thinkingDuration: props.thinkingDuration,
    toolCalls: legacyToolCalls,
  });
});

const hasVisibleCompletedThinkingContent = computed(() =>
  !shouldHideThinkingBlocks()
  && canonicalLiveRenderParts.value.some((part) =>
    part.kind === "thinking" && !part.active && part.content.trim(),
  ),
);
const hasLiveToolCalls = computed(() => props.activeToolCalls.length > 0);
const hasTransientToolCalls = computed(() => transientToolCalls.value.length > 0);
const hasToolCallHandoff = computed(() => hasTransientToolCalls.value && !hasLiveToolCalls.value);
const hasVisibleActiveThinkingBlock = computed(() =>
  props.isThinking
  && canonicalLiveRenderParts.value.some((part) => part.kind === "thinking" && part.active),
);
const hasVisibleTransientThinkingBlock = computed(
  () => hasVisibleActiveThinkingBlock.value || hasVisibleCompletedThinkingContent.value,
);
const hasStreamingContent = computed(() =>
  hasVisibleStreamingText.value
  || canonicalLiveRenderParts.value.some((part) => part.kind === "text" || part.kind === "toolCall")
  || hasLiveToolCalls.value,
);
const isWaitingForResponse = computed(
  () => shouldShowWaitingPlaceholder({
    isStreaming: props.isStreaming,
    hasStreamingContent: hasStreamingContent.value,
    isThinking: props.isThinking,
    hasThinkingContent: hasVisibleCompletedThinkingContent.value,
  }),
);
const isToolWaitingForResponse = computed(() => isWaitingForResponse.value && hasTransientToolCalls.value);
const isToolWaitingRowVisible = computed(() => isToolWaitingForResponse.value && !hasToolCallHandoff.value);
const isToolWaitingStatusVisible = computed(() => isToolWaitingForResponse.value && hasToolCallHandoff.value);
const isStandaloneWaitingPlaceholder = computed(() => isWaitingForResponse.value && !hasTransientToolCalls.value);
const effectiveWaitingLabel = computed(() => props.isCompacting ? props.compactingLabel : props.waitingLabel);
const hasStandaloneCompactingPlaceholder = computed(() => props.isCompacting && !isWaitingForResponse.value);
const hasTransientAssistantMessage = computed(
  () =>
    hasStreamingContent.value
    || hasToolCallHandoff.value
    || hasVisibleTransientThinkingBlock.value
    || isWaitingForResponse.value
    || hasStandaloneCompactingPlaceholder.value,
);

watch(
  () => `${Number(isStandaloneWaitingPlaceholder.value)}:${Number(isToolWaitingForResponse.value)}:${Number(isToolWaitingRowVisible.value)}:${Number(isToolWaitingStatusVisible.value)}:${transientToolCalls.value.length}`,
  (next, previous) => {
    traceToolLayoutChange("waitingLayoutStateChanged", {
      previous,
      next,
      standaloneWaiting: isStandaloneWaitingPlaceholder.value,
      toolWaiting: isToolWaitingForResponse.value,
      toolWaitingRowVisible: isToolWaitingRowVisible.value,
      toolWaitingStatusVisible: isToolWaitingStatusVisible.value,
      transientToolCallCount: transientToolCalls.value.length,
    });
    traceToolCollapse("waitingLayoutStateChanged", {
      previous,
      next,
      standaloneWaiting: isStandaloneWaitingPlaceholder.value,
      toolWaiting: isToolWaitingForResponse.value,
      toolWaitingRowVisible: isToolWaitingRowVisible.value,
      toolWaitingStatusVisible: isToolWaitingStatusVisible.value,
      transientToolCallCount: transientToolCalls.value.length,
      hasHandoff: !!toolCallHandoff.value,
      streamingTextLen: props.streamingText.length,
      isStreaming: props.isStreaming,
    });
  },
);

const isStreamingContinuation = computed(() => {
  const groups = groupedMessages.value;
  return shouldShowAssistantContinuation(
    groups.length > 0 ? groups[groups.length - 1].role : null,
    hasTransientAssistantMessage.value,
  );
});

const pendingContinuationToolItemIds = computed(() => {
  if (toolCallHandoff.value?.collapseArmed) {
    return new Set<string>();
  }

  const groups = baseGroupedMessages.value;
  const lastGroup = groups[groups.length - 1];

  const segments =
    lastGroup?.role === "assistant"
      ? historyRenderSegmentsForGroup(lastGroup).map((segment): PendingContinuationRenderSegment => ({
          type: segment.type === "toolCalls" || segment.type === "content" ? segment.type : "other",
          itemIds: segment.type === "toolCalls" ? segment.itemIds : undefined,
        }))
      : [];

  return collectPendingContinuationToolSegmentItemIds({
    isStreaming: props.isStreaming,
    lastGroupRole: lastGroup?.role ?? null,
    hasTransientAssistantMessage: hasTransientAssistantMessage.value,
    segments,
  });
});

function pendingContinuationSegmentSnapshot() {
  const groups = baseGroupedMessages.value;
  const lastGroup = groups[groups.length - 1];
  const segments =
    lastGroup?.role === "assistant"
      ? historyRenderSegmentsForGroup(lastGroup).map((segment, index) => ({
          index,
          type: segment.type === "toolCalls" || segment.type === "content" ? segment.type : "other",
          itemIds: segment.type === "toolCalls" ? segment.itemIds : [],
        }))
      : [];

  return {
    lastGroupRole: lastGroup?.role ?? null,
    segments,
  };
}

watch(
  () => Array.from(pendingContinuationToolItemIds.value).join("\u241f"),
  (next, previous) => {
    const snapshot = pendingContinuationSegmentSnapshot();
    traceToolLayoutChange("pendingContinuationToolItemIdsChanged", {
      previous: previous ? previous.split("\u241f") : [],
      next: next ? next.split("\u241f") : [],
      lastGroupRole: snapshot.lastGroupRole,
      historySegments: snapshot.segments,
    });
    traceToolCollapse("pendingContinuationToolItemIdsChanged", {
      previous: previous ? previous.split("\u241f") : [],
      next: next ? next.split("\u241f") : [],
      isStreaming: props.isStreaming,
      hasTransientAssistantMessage: hasTransientAssistantMessage.value,
      activeToolCallCount: props.activeToolCalls.length,
      activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
      hasHandoff: !!toolCallHandoff.value,
      handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
      handoffCollapseFinished: toolCallHandoff.value?.collapseFinished ?? false,
      lastGroupRole: snapshot.lastGroupRole,
      historySegments: snapshot.segments,
    });
  },
);

const nonCollapsibleToolItemIds = computed(() => {
  return new Set(pendingContinuationToolItemIds.value);
});

function shouldKeepToolItemExpanded(itemId: string) {
  return nonCollapsibleToolItemIds.value.has(itemId);
}

function markdownUnityPreviewStateScope(segment: { key: string; itemId?: string }) {
  return [
    props.variant,
    props.sessionKey?.trim() || "__draft__",
    segment.itemId ?? "__transient__",
    segment.key,
  ].join(":");
}

type HistoryRenderSegment =
  | { type: "thinking"; key: string; part: AssistantRenderPart; itemId: string; content: string; duration?: number }
  | { type: "toolCalls"; key: string; part: Extract<AssistantRenderPart, { kind: "toolCall" }>; itemId: string; itemIds: string[]; toolCalls: ToolCallDisplay[] }
  | { type: "content"; key: string; part: AssistantRenderPart; itemId: string; content: string }
  | { type: "knowledgeProposal"; key: string; part: AssistantRenderPart; itemId: string; message: ChatMessage };

type TransientRenderSegment =
  | { type: "thinking"; key: string; part: AssistantRenderPart; active: boolean; duration?: number }
  | {
      type: "toolCalls";
      key: string;
      part: Extract<AssistantRenderPart, { kind: "toolCall" }>;
      toolCalls: ToolCallDisplay[];
      showWaiting: boolean;
      allowCollapse: boolean;
      collapseEnabled: boolean;
      animateCollapseOnMount: boolean;
    }
  | { type: "waiting"; key: string; label: string }
  | { type: "content"; key: string; part: AssistantRenderPart; content: string };

function renderPartsForMessage(item: MessageRenderItem): AssistantRenderPart[] {
  const hasToolFilter = hasExplicitDisplayToolCalls(item) || !!toolCallInfosForMessage(item.message);
  const messageToolCalls = resolveToolCallInfosForRender({
    messageToolCalls: toolCallInfosForMessage(item.message),
    displayToolCalls: item.displayToolCalls,
  });
  const hasRenderPartThinking = !!item.message.renderParts?.some((part) =>
    part.kind === "thinking" && part.content.trim().length > 0,
  );
  const shouldRenderThinkingPart =
    shouldRenderHistoryThinkingBlock(item)
    || (!shouldHideThinkingBlocks() && hasRenderPartThinking);

  let parts: AssistantRenderPart[];
  if (item.message.renderParts?.length) {
    assertCanonicalRenderParts(item.message.renderParts, `message:${item.message.id}`);
    const visibleToolIds = new Set((messageToolCalls ?? []).map((toolCall) => toolCall.id));
    parts = [...item.message.renderParts]
      .filter((part) => part.kind !== "thinking" || shouldRenderThinkingPart)
      .filter((part) => part.kind !== "toolCall" || !hasToolFilter || visibleToolIds.has(part.toolCall.id))
      .sort(compareAssistantRenderParts);
  } else {
    parts = synthesizeLegacyRenderParts(item.message, {
      toolCalls: messageToolCalls,
      beforeContentToolCalls: item.displayToolCallsBeforeContent,
      afterContentToolCalls: item.displayToolCallsAfterContent,
      knowledgeProposals: item.attachedKnowledgeProposals,
    }).filter((part) => part.kind !== "thinking" || shouldRenderThinkingPart);
  }

  if (!item.message.renderParts?.length || item.attachedKnowledgeProposals.length === 0) {
    return parts;
  }

  const lastOrder = parts[parts.length - 1]?.order ?? { runId: "legacy", seq: 1 };
  return [
    ...parts,
    ...item.attachedKnowledgeProposals
      .filter((message) => !!message.knowledgeProposal)
      .map((message, index): AssistantRenderPart => ({
        kind: "knowledgeProposal",
        id: message.id,
        order: { runId: lastOrder.runId, seq: lastOrder.seq + 100 + index },
        message,
      })),
  ].sort(compareAssistantRenderParts);
}

function toolCallDisplayForPart(part: Extract<AssistantRenderPart, { kind: "toolCall" }>) {
  return toolCallsFromInfos([part.toolCall])[0];
}

function toolCallInfoFromDisplay(toolCall: ToolCallDisplay): ToolCallInfo {
  return {
    id: toolCall.id,
    name: toolCall.name,
    arguments: toolCall.arguments,
    order: toolCall.order,
    outcome: toolCall.status === "running" ? undefined : toolCall.status,
    recordedOutput: toolCall.output,
    nestedToolCalls: toolCall.nestedToolCalls?.map((nestedToolCall) => toolCallInfoFromDisplay(nestedToolCall)),
  };
}

function transientToolHandoffPart(toolCalls: ToolCallDisplay[]): Extract<AssistantRenderPart, { kind: "toolCall" }> | null {
  const firstToolCall = toolCalls[0];
  if (!firstToolCall) return null;

  return {
    kind: "toolCall",
    id: "transient:tools-handoff",
    order: { runId: "legacy", seq: firstToolCallRenderOrder(toolCalls) || 1 },
    toolCall: toolCallInfoFromDisplay(firstToolCall),
  };
}

function transientToolSegmentKey(toolCalls: ToolCallDisplay[]) {
  return `transient:tools:${toolCalls[0]?.id ?? "handoff"}`;
}

function historyToolSegmentKey(toolCalls: ToolCallDisplay[], fallbackId: string) {
  return `history:tools:${toolCalls[0]?.id ?? fallbackId}`;
}

function historyRenderSegments(item: MessageRenderItem): HistoryRenderSegment[] {
  const segments: HistoryRenderSegment[] = [];
  let pendingToolPart: Extract<AssistantRenderPart, { kind: "toolCall" }> | null = null;
  let pendingToolCalls: ToolCallDisplay[] = [];
  let pendingToolPartIds: string[] = [];

  const flushPendingTools = () => {
    if (!pendingToolPart || pendingToolCalls.length === 0) return;
    segments.push({
      type: "toolCalls",
      key: historyToolSegmentKey(pendingToolCalls, pendingToolPart.id),
      part: pendingToolPart,
      itemId: item.id,
      itemIds: [item.id],
      toolCalls: pendingToolCalls,
    });
    pendingToolPart = null;
    pendingToolCalls = [];
    pendingToolPartIds = [];
  };

  for (const part of renderPartsForMessage(item)) {
    if (part.kind === "thinking") {
      flushPendingTools();
      segments.push({
        type: "thinking",
        key: `${item.id}:${part.id}`,
        part,
        itemId: item.id,
        content: part.content,
        duration: part.duration,
      });
    } else if (part.kind === "text") {
      flushPendingTools();
      segments.push({
        type: "content",
        key: `${item.id}:${part.id}`,
        part,
        itemId: item.id,
        content: part.content,
      });
    } else if (part.kind === "toolCall") {
      const toolCall = toolCallDisplayForPart(part);
      if (toolCall) {
        pendingToolPart ??= part;
        pendingToolPartIds.push(part.id);
        pendingToolCalls.push(toolCall);
      }
    } else if (part.kind === "knowledgeProposal") {
      flushPendingTools();
      segments.push({
        type: "knowledgeProposal",
        key: `${item.id}:${part.id}`,
        part,
        itemId: item.id,
        message: part.message,
      });
    }
  }
  flushPendingTools();
  return segments;
}

function historyRenderSegmentsForGroup(group: MessageGroup): HistoryRenderSegment[] {
  if (group.role !== "assistant") return [];

  const segments: HistoryRenderSegment[] = [];
  let pendingToolPart: Extract<AssistantRenderPart, { kind: "toolCall" }> | null = null;
  let pendingToolCalls: ToolCallDisplay[] = [];
  let pendingToolPartIds: string[] = [];
  let pendingToolItemIds: string[] = [];

  const flushPendingTools = () => {
    if (!pendingToolPart || pendingToolCalls.length === 0) return;
    const itemIds = Array.from(new Set(pendingToolItemIds));
    segments.push({
      type: "toolCalls",
      key: historyToolSegmentKey(pendingToolCalls, pendingToolPart.id),
      part: pendingToolPart,
      itemId: itemIds[0] ?? group.id,
      itemIds,
      toolCalls: pendingToolCalls,
    });
    pendingToolPart = null;
    pendingToolCalls = [];
    pendingToolPartIds = [];
    pendingToolItemIds = [];
  };

  for (const item of group.items) {
    for (const segment of historyRenderSegments(item)) {
      if (segment.type === "toolCalls") {
        pendingToolPart ??= segment.part;
        pendingToolPartIds.push(segment.part.id);
        pendingToolItemIds.push(...segment.itemIds);
        pendingToolCalls.push(...segment.toolCalls);
      } else {
        flushPendingTools();
        segments.push(segment);
      }
    }
  }
  flushPendingTools();
  return segments;
}

function isToolOnlyMessageGroup(group: MessageGroup) {
  if (group.role !== "assistant") return false;
  const segments = historyRenderSegmentsForGroup(group);
  return segments.length > 0 && segments.every((segment) => segment.type === "toolCalls");
}

function shouldKeepToolSegmentExpanded(segment: Extract<HistoryRenderSegment, { type: "toolCalls" }>) {
  if (!segment.itemIds.some((itemId) => shouldKeepToolItemExpanded(itemId))) return false;
  return !areToolCallDisplaysCoveredByMatchState(
    segment.toolCalls,
    retainedCollapsedToolCallMatchState.value,
  );
}

function historyToolSegmentPinnedSnapshot() {
  return groupedMessages.value.flatMap((group, groupIndex) => {
    if (group.role !== "assistant") return [];
    return historyRenderSegmentsForGroup(group)
      .filter((segment): segment is Extract<HistoryRenderSegment, { type: "toolCalls" }> => segment.type === "toolCalls")
      .map((segment, segmentIndex) => ({
        groupIndex,
        segmentIndex,
        key: segment.key,
        itemIds: segment.itemIds,
        toolCallIds: segment.toolCalls.map((toolCall) => toolCall.id),
        keepExpanded: shouldKeepToolSegmentExpanded(segment),
      }))
      .filter((segment) => segment.keepExpanded);
  });
}

watch(
  () => JSON.stringify(historyToolSegmentPinnedSnapshot()),
  (next, previous) => {
    traceToolLayoutChange("historyToolSegmentPinnedStateChanged", {
      previous: previous ? JSON.parse(previous) : [],
      next: next ? JSON.parse(next) : [],
      pendingItemIds: Array.from(pendingContinuationToolItemIds.value),
    });
    traceToolCollapse("historyToolSegmentPinnedStateChanged", {
      previous: previous ? JSON.parse(previous) : [],
      next: next ? JSON.parse(next) : [],
      pendingItemIds: Array.from(pendingContinuationToolItemIds.value),
      isStreaming: props.isStreaming,
      hasTransientAssistantMessage: hasTransientAssistantMessage.value,
      activeToolCallCount: props.activeToolCalls.length,
      activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
      hasHandoff: !!toolCallHandoff.value,
      handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
      handoffCollapseFinished: toolCallHandoff.value?.collapseFinished ?? false,
    });
  },
);

function historyToolSegmentExpansionSnapshot() {
  return groupedMessages.value.flatMap((group, groupIndex) => {
    if (group.role !== "assistant") return [];
    return historyRenderSegmentsForGroup(group)
      .filter((segment): segment is Extract<HistoryRenderSegment, { type: "toolCalls" }> => segment.type === "toolCalls")
      .map((segment, segmentIndex) => {
        const pendingContinuation = segment.itemIds.some((itemId) => shouldKeepToolItemExpanded(itemId));
        const retainedCovered = areToolCallDisplaysCoveredByMatchState(
          segment.toolCalls,
          retainedCollapsedToolCallMatchState.value,
        );
        return {
          groupIndex,
          segmentIndex,
          key: segment.key,
          itemIds: segment.itemIds,
          toolCallIds: segment.toolCalls.map((toolCall) => toolCall.id),
          pendingContinuation,
          retainedCovered,
          keepExpanded: pendingContinuation && !retainedCovered,
        };
      });
  });
}

watch(
  () => JSON.stringify(historyToolSegmentExpansionSnapshot()),
  (next, previous) => {
    traceToolLayoutChange("historyToolSegmentExpansionDecision", {
      previous: previous ? JSON.parse(previous) : [],
      next: next ? JSON.parse(next) : [],
      retainedEntryCount: toolCallMatchEntryCount(retainedCollapsedToolCallMatchState.value),
    });
    traceToolCollapse("historyToolSegmentExpansionDecision", {
      previous: previous ? JSON.parse(previous) : [],
      next: next ? JSON.parse(next) : [],
      pendingItemIds: Array.from(pendingContinuationToolItemIds.value),
      retainedEntryCount: toolCallMatchEntryCount(retainedCollapsedToolCallMatchState.value),
      isStreaming: props.isStreaming,
      hasTransientAssistantMessage: hasTransientAssistantMessage.value,
      activeToolCallCount: props.activeToolCalls.length,
      activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
      hasHandoff: !!toolCallHandoff.value,
      handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
      handoffCollapseFinished: toolCallHandoff.value?.collapseFinished ?? false,
    });
  },
);

const transientRenderSegments = computed<TransientRenderSegment[]>(() => {
  const segments: TransientRenderSegment[] = [];
  const transientById = new Map(transientToolCalls.value.map((toolCall) => [toolCall.id, toolCall]));
  let pendingToolPart: Extract<AssistantRenderPart, { kind: "toolCall" }> | null = null;
  let pendingToolCalls: ToolCallDisplay[] = [];
  let pendingToolPartIds: string[] = [];

  const flushPendingTools = () => {
    if (!pendingToolPart || pendingToolCalls.length === 0) return;
    segments.push({
      type: "toolCalls",
      key: transientToolSegmentKey(pendingToolCalls),
      part: pendingToolPart,
      toolCalls: pendingToolCalls,
      showWaiting: false,
      allowCollapse: transientToolCallsAllowCollapse.value,
      collapseEnabled: transientToolCallsCollapseEnabled.value,
      animateCollapseOnMount: !!toolCallHandoff.value?.collapseArmed,
    });
    pendingToolPart = null;
    pendingToolCalls = [];
    pendingToolPartIds = [];
  };

  for (const part of canonicalLiveRenderParts.value) {
    if (part.kind === "thinking") {
      if (!shouldRenderTransientThinkingSegment(part)) continue;
      flushPendingTools();
      segments.push({
        type: "thinking",
        key: `transient:${part.id}`,
        part,
        active: !!part.active && props.isThinking,
        duration: part.duration ?? props.thinkingDuration,
      });
    } else if (part.kind === "text") {
      flushPendingTools();
      const content = displayedLiveRenderParts.value.length <= 1 && props.streamingText ? props.streamingText : part.content;
      if (content) {
        segments.push({
          type: "content",
          key: `transient:${part.id}`,
          part,
          content,
        });
      }
    } else if (part.kind === "toolCall") {
      const toolCall = transientById.get(part.toolCall.id) ?? toolCallDisplayForPart(part);
      if (toolCall) {
        pendingToolPart ??= part;
        pendingToolPartIds.push(part.id);
        pendingToolCalls.push(toolCall);
      }
    }
  }
  flushPendingTools();

  let hasRenderedToolSegment = segments.some((segment) => segment.type === "toolCalls");
  const promotedToolCalls = promotableHistoryToolCalls.value.toolCalls;
  if (shouldRenderPromotedHistoryToolCallsInTransient.value && promotedToolCalls.length > 0) {
    const promotedPart = transientToolHandoffPart(transientToolCalls.value);
    if (promotedPart) {
      const firstToolSegmentIndex = segments.findIndex((segment) => segment.type === "toolCalls");
      const firstContentSegmentIndex = segments.findIndex((segment) => segment.type === "content");
      const candidateToolSegment = firstToolSegmentIndex >= 0 ? segments[firstToolSegmentIndex] : null;
      const firstToolSegment = candidateToolSegment?.type === "toolCalls" ? candidateToolSegment : null;
      const firstToolPrecedesContent =
        !!firstToolSegment
        && (firstContentSegmentIndex < 0 || firstToolSegmentIndex < firstContentSegmentIndex);
      const shouldCollapsePromotedPrefix =
        !!toolCallHandoff.value?.collapseArmed
        || firstContentSegmentIndex >= 0;

      if (firstToolPrecedesContent) {
        const mergedToolCalls = mergeToolCallDisplaysWithoutDuplicates(
          promotedToolCalls,
          firstToolSegment.toolCalls,
        );
        firstToolSegment.key = transientToolSegmentKey(mergedToolCalls);
        firstToolSegment.part = promotedPart;
        firstToolSegment.toolCalls = mergedToolCalls;
        firstToolSegment.allowCollapse = true;
        firstToolSegment.collapseEnabled = shouldCollapsePromotedPrefix;
      } else {
        const prefixToolCalls =
          firstToolSegmentIndex < 0 && transientOwnedToolCalls.value.length > 0
            ? transientToolCalls.value
            : promotedToolCalls;
        segments.unshift({
          type: "toolCalls",
          key: transientToolSegmentKey(prefixToolCalls),
          part: promotedPart,
          toolCalls: prefixToolCalls,
          showWaiting: false,
          allowCollapse: true,
          collapseEnabled: shouldCollapsePromotedPrefix,
          animateCollapseOnMount: !!toolCallHandoff.value?.collapseArmed,
        });
      }
    }
  }
  hasRenderedToolSegment = segments.some((segment) => segment.type === "toolCalls");

  if (!hasRenderedToolSegment && transientToolCalls.value.length > 0) {
    const handoffPart = transientToolHandoffPart(transientToolCalls.value);
    if (handoffPart) {
      segments.unshift({
        type: "toolCalls",
        key: transientToolSegmentKey(transientToolCalls.value),
        part: handoffPart,
        toolCalls: transientToolCalls.value,
        showWaiting: false,
        allowCollapse: transientToolCallsAllowCollapse.value,
        collapseEnabled: transientToolCallsCollapseEnabled.value,
        animateCollapseOnMount: !!toolCallHandoff.value?.collapseArmed,
      });
    }
  }

  const toolSegments = segments.filter((segment) => segment.type === "toolCalls");
  const lastToolSegment = toolSegments[toolSegments.length - 1];
  if (lastToolSegment) {
    lastToolSegment.showWaiting = true;
  }

  if (shouldRenderPromotedHistoryToolCallsInTransient.value) {
    const promotedIds = promotableHistoryToolCalls.value.toolCalls.map((toolCall) => toolCall.id);
    const renderedIds = toolSegments.flatMap((segment) => segment.toolCalls.map((toolCall) => toolCall.id));
    const renderedIdSet = new Set(renderedIds);
    const missingPromotedIds = promotedIds.filter((id) => !renderedIdSet.has(id));
    const detail = {
      promotedToolCallCount: promotedIds.length,
      promotedToolCallIds: promotedIds,
      transientToolCallCount: transientToolCalls.value.length,
      transientToolCallIds: transientToolCalls.value.map((toolCall) => toolCall.id),
      renderedToolSegmentCount: toolSegments.length,
      renderedToolCallIds: renderedIds,
      missingPromotedToolCallIds: missingPromotedIds,
      hasRenderedToolSegment,
      activeToolCallCount: props.activeToolCalls.length,
      activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
      livePartKinds: canonicalLiveRenderParts.value.map((part) => part.kind),
      livePartIds: canonicalLiveRenderParts.value.map((part) => part.id),
      hasHandoff: !!toolCallHandoff.value,
      handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
      isStreaming: props.isStreaming,
    };
    traceToolCollapse("transientPromotedToolCallsCoverage", detail);
    if (missingPromotedIds.length > 0) {
      traceToolCollapse("promotedHistoryToolCallsRenderGap", detail);
    }
  }

  if (hasStandaloneCompactingPlaceholder.value || isStandaloneWaitingPlaceholder.value) {
    const isCompactingPlaceholder = props.isCompacting;
    segments.push({
      type: "waiting",
      key: isCompactingPlaceholder ? "transient:compacting" : "transient:waiting",
      label: isCompactingPlaceholder ? props.compactingLabel : props.waitingLabel,
    });
  }

  return segments;
});

// The concrete tool calls the transient region renders this tick. Drives the
// fail-open hide decision in `historyHiddenToolCallMatchState` above so history
// only hides what transient actually shows.
const transientRenderedToolCalls = computed(() =>
  collectRenderedToolCalls(
    transientRenderSegments.value.filter(
      (segment): segment is Extract<TransientRenderSegment, { type: "toolCalls" }> =>
        segment.type === "toolCalls",
    ),
  ),
);

function transientSegmentPaintState() {
  return transientRenderSegments.value.map((segment, index) => {
    if (segment.type === "thinking") {
      return {
        index,
        type: segment.type,
        key: segment.key,
        active: segment.active,
        duration: segment.duration ?? 0,
      };
    }
    if (segment.type === "toolCalls") {
      return {
        index,
        type: segment.type,
        key: segment.key,
        toolCallCount: segment.toolCalls.length,
        toolCallIds: segment.toolCalls.map((toolCall) => toolCall.id),
        statuses: toolLayoutStatuses(segment.toolCalls),
        showWaiting: segment.showWaiting,
        allowCollapse: segment.allowCollapse,
        collapseEnabled: segment.collapseEnabled,
        animateCollapseOnMount: segment.animateCollapseOnMount,
      };
    }
    if (segment.type === "content") {
      return {
        index,
        type: segment.type,
        key: segment.key,
        textLength: segment.content.length,
        textPreview: previewTraceText(segment.content, 48),
      };
    }
    return {
      index,
      type: segment.type,
      key: segment.key,
      label: segment.label,
    };
  });
}

function historyToolBlockOrderState() {
  return groupedMessages.value.flatMap((group, groupIndex) =>
    historyRenderSegmentsForGroup(group)
      .map((segment, segmentIndex) => {
        if (segment.type !== "toolCalls") return null;
        return {
          groupIndex,
          groupId: group.id,
          role: group.role,
          segmentIndex,
          key: segment.key,
          itemIds: segment.itemIds,
          toolCallCount: segment.toolCalls.length,
          toolCallIds: segment.toolCalls.map((toolCall) => toolCall.id),
          statuses: toolLayoutStatuses(segment.toolCalls),
          keepExpanded: shouldKeepToolSegmentExpanded(segment),
        };
      })
      .filter((segment): segment is NonNullable<typeof segment> => !!segment));
}

function transcriptGroupOrderState() {
  return groupedMessages.value.map((group, groupIndex) => ({
    groupIndex,
    id: group.id,
    role: group.role,
    itemIds: group.items.map((item) => item.id),
    itemRoles: group.items.map((item) => item.message.role),
    itemContentLens: group.items.map((item) => item.message.content.length),
    itemToolCallIds: group.items.map((item) => item.message.toolCalls?.map((toolCall) => toolCall.id) ?? []),
    segmentOrder: historyRenderSegmentsForGroup(group).map((segment, segmentIndex) => {
      if (segment.type === "toolCalls") {
        return {
          segmentIndex,
          type: segment.type,
          key: segment.key,
          itemIds: segment.itemIds,
          toolCallIds: segment.toolCalls.map((toolCall) => toolCall.id),
        };
      }
      if (segment.type === "content") {
        return {
          segmentIndex,
          type: segment.type,
          key: segment.key,
          contentLen: segment.content.length,
          contentPreview: previewTraceText(segment.content, 48),
        };
      }
      if (segment.type === "knowledgeProposal") {
        return {
          segmentIndex,
          type: segment.type,
          key: segment.key,
          messageId: segment.message.id,
        };
      }
      return {
        segmentIndex,
        type: segment.type,
        key: segment.key,
      };
    }),
  }));
}

function transcriptBlockOrderState() {
  const historyToolBlocks = historyToolBlockOrderState();
  const transientSegments = transientSegmentPaintState();
  const historyToolIds = new Set(historyToolBlocks.flatMap((block) => block.toolCallIds));
  const transientToolIds = transientSegments.flatMap((segment) =>
    segment.type === "toolCalls" ? segment.toolCallIds : []);

  return {
    sessionKey: props.sessionKey ?? "",
    isStreaming: props.isStreaming,
    hasTransientAssistantMessage: hasTransientAssistantMessage.value,
    activeToolCalls: props.activeToolCalls.map((toolCall) => ({
      id: toolCall.id,
      name: toolCall.name,
      status: toolCall.status,
      order: toolCall.order ?? null,
    })),
    hasHandoff: !!toolCallHandoff.value,
    handoff: toolCallHandoff.value
      ? {
        renderKey: toolCallHandoff.value.renderKey,
        toolCallIds: Array.from(toolCallHandoff.value.toolCallIds),
        collapseArmed: toolCallHandoff.value.collapseArmed,
        collapseFinished: toolCallHandoff.value.collapseFinished,
      }
      : null,
    shouldHidePromotedHistoryToolCalls: shouldHidePromotedHistoryToolCalls.value,
    shouldRenderPromotedHistoryToolCallsInTransient: shouldRenderPromotedHistoryToolCallsInTransient.value,
    promotedHistoryToolCallIds: Array.from(promotableHistoryToolCalls.value.toolCallIds),
    duplicateHistoryTransientToolIds: transientToolIds.filter((id, index) =>
      transientToolIds.indexOf(id) === index && historyToolIds.has(id)),
    groups: transcriptGroupOrderState(),
    historyToolBlocks,
    transientSegments,
  };
}

watch(
  () => JSON.stringify(historyToolBlockOrderState()),
  (next, previous) => {
    traceToolCollapse("historyToolBlockOrderChanged", {
      previous: parseTraceJson(previous),
      next: parseTraceJson(next),
      blockOrder: parseTraceJson(next),
      transcriptOrder: transcriptBlockOrderState(),
    });
  },
  { flush: "post", immediate: true },
);

watch(
  () => JSON.stringify(transientSegmentPaintState()),
  (next, previous) => {
    traceToolCollapse("transientRenderSegmentsChanged", {
      previous: parseTraceJson(previous),
      next: parseTraceJson(next),
      transcriptOrder: transcriptBlockOrderState(),
    });
  },
  { flush: "post", immediate: true },
);

watch(
  () => JSON.stringify(transcriptBlockOrderState()),
  (next, previous) => {
    const nextState = parseTraceJson(next);
    traceToolLayoutChange("transcriptBlockOrderChanged", {
      previous: parseTraceJson(previous),
      next: nextState,
    });
    traceToolCollapse("transcriptBlockOrderChanged", {
      previous: parseTraceJson(previous),
      next: nextState,
    });
  },
  { flush: "post", immediate: true },
);

function hasTransientStatusPaintTarget() {
  return hasVisibleActiveThinkingBlock.value
    || isStandaloneWaitingPlaceholder.value
    || hasStandaloneCompactingPlaceholder.value
    || isToolWaitingStatusVisible.value;
}

function traceTransientStatusPaint(reason: string, detail: Record<string, unknown> = {}) {
  if (!hasTransientStatusPaintTarget()) return;
  traceTranscriptPaintOcclusion({
    scope: `chat-transcript:${props.variant}`,
    reason,
    scrollElement: scrollRef.value,
    contentElement: contentRef.value,
    detail: {
      sessionKey: props.sessionKey ?? "",
      isStreaming: props.isStreaming,
      isThinking: props.isThinking,
      hasVisibleActiveThinkingBlock: hasVisibleActiveThinkingBlock.value,
      activeToolCallCount: props.activeToolCalls.length,
      activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
      activeToolCallStatuses: toolLayoutStatuses(props.activeToolCalls),
      hasHandoff: !!toolCallHandoff.value,
      handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
      handoffCollapseFinished: toolCallHandoff.value?.collapseFinished ?? false,
      standaloneWaiting: isStandaloneWaitingPlaceholder.value,
      compactingWaiting: hasStandaloneCompactingPlaceholder.value,
      toolWaiting: isToolWaitingForResponse.value,
      toolWaitingRowVisible: isToolWaitingRowVisible.value,
      toolWaitingStatusVisible: isToolWaitingStatusVisible.value,
      transientSegments: transientSegmentPaintState(),
      liveRenderParts: canonicalLiveRenderParts.value.map((part) => ({
        kind: part.kind,
        id: part.id,
        active: part.kind === "thinking" ? part.active : undefined,
        order: part.order ?? 0,
      })),
      ...detail,
    },
  });
}

watch(
  () => [
    Number(hasVisibleActiveThinkingBlock.value),
    Number(props.isThinking),
    Number(isStandaloneWaitingPlaceholder.value),
    Number(hasStandaloneCompactingPlaceholder.value),
    Number(isToolWaitingForResponse.value),
    Number(isToolWaitingRowVisible.value),
    Number(isToolWaitingStatusVisible.value),
    props.activeToolCalls.map((toolCall) => `${toolCall.id}:${toolCall.status}`).join(","),
    transientSegmentPaintState().map((segment) => JSON.stringify(segment)).join("|"),
  ].join("::"),
  (next, previous) => {
    traceTransientStatusPaint("transientStatusPaintStateChanged", {
      previous,
      next,
    });
  },
  { flush: "post", immediate: true },
);

function formatThoughtSummary(duration?: number) {
  if (duration && duration > 0) {
    return props.thoughtDurationLabel.replace("{0}", String(duration));
  }
  return props.thoughtMomentLabel;
}

function messageGroupLabel(group: MessageGroup) {
  return group.items.some((item) => isCompactHandoffMessage(item.message))
    ? props.handoffLabel
    : group.role === "user"
      ? props.userLabel
      : props.assistantLabel;
}

function isCompactMarkerGroup(group: MessageGroup) {
  return group.items.length > 0 && group.items.every((item) => isCompactHandoffMessage(item.message));
}

function shouldRightAlignUserMessageGroup(group: Pick<MessageGroup, "role">) {
  return props.variant === "session" && displaySettings.rightAlignUserMessages && group.role === "user";
}

function isContextSelectedMessage(messageId: string | null | undefined) {
  return !!messageId && props.selectedMessageId === messageId;
}

function isContextSelectedAssistantGroup(group: MessageGroup) {
  return group.role === "assistant" && group.items.some((item) => isContextSelectedMessage(item.id));
}

function shouldShowSessionRoundDivider(group: Pick<MessageGroup, "role">, index: number) {
  if (props.variant !== "session" || group.role !== "user" || index <= 0) return false;
  const previousGroup = groupedMessages.value[index - 1];
  return !previousGroup || !isCompactMarkerGroup(previousGroup);
}

const hasFooterSlot = computed(() => !!slots.footer);
const showSessionFooter = computed(
  () =>
    props.variant === "session"
    && hasFooterSlot.value
    && (groupedMessages.value.length > 0 || hasTransientAssistantMessage.value),
);

function imageDataUrl(message: ChatMessage, index: number) {
  const image = message.images?.[index];
  if (!image) return "";
  return `data:${image.mimeType};base64,${image.data}`;
}

function emitScroll(event: Event) {
  emit("scroll", event);
}

const SCROLL_INTENT_KEYS = new Set([
  "ArrowDown",
  "ArrowUp",
  "End",
  "Home",
  "PageDown",
  "PageUp",
  " ",
  "Spacebar",
]);

function emitUserScrollIntent(event: Event) {
  emit("userScrollIntent", event);
}

function emitPointerScrollIntent(event: PointerEvent) {
  if (event.target === scrollRef.value) {
    emit("userScrollIntent", event);
  }
}

function emitKeyboardScrollIntent(event: KeyboardEvent) {
  if (SCROLL_INTENT_KEYS.has(event.key)) {
    emit("userScrollIntent", event);
  }
}

function emitContentClick(event: MouseEvent) {
  emit("contentClick", event);
}

function emitContentContextmenu(event: MouseEvent) {
  emit("contentContextmenu", event);
}

function emitToolViewportAnchorStart(anchor: HTMLElement) {
  emit("toolViewportAnchorStart", anchor);
}

function emitToolViewportAnchorEnd(anchor: HTMLElement) {
  emit("toolViewportAnchorEnd", anchor);
}

function openImage(src: string) {
  if (!src) return;
  emit("openImage", src);
}
</script>

<template>
  <div
    ref="scrollRef"
    class="chat-transcript-scroll"
    :class="`is-${variant}`"
    @scroll="emitScroll"
    @wheel.passive="emitUserScrollIntent"
    @touchstart.passive="emitUserScrollIntent"
    @pointerdown="emitPointerScrollIntent"
    @keydown="emitKeyboardScrollIntent"
    @click="emitContentClick"
    @contextmenu="emitContentContextmenu"
  >
    <div ref="contentRef" class="chat-transcript-content">
      <div
        v-if="groupedMessages.length === 0 && !hasTransientAssistantMessage"
        class="chat-transcript-empty"
        :class="`is-${variant}`"
      >
        <slot name="empty">
          <div v-if="emptyTitle" class="chat-transcript-empty-title">{{ emptyTitle }}</div>
          <div v-if="emptyHint" class="chat-transcript-empty-hint">{{ emptyHint }}</div>
        </slot>
      </div>
      <template v-else>
        <template
          v-for="(group, idx) in groupedMessages"
          :key="group.id"
        >
          <div
            v-if="isCompactMarkerGroup(group)"
            v-memo="[group, idx, historyGroupMemoKey(group, idx)]"
            class="chat-transcript-compact-marker"
            :class="`is-${variant}`"
            :data-scroll-anchor-id="group.items[0]?.id"
          >
            <span class="chat-transcript-compact-marker-line"></span>
            <span class="chat-transcript-compact-marker-label">{{ compactedLabel }}</span>
            <span class="chat-transcript-compact-marker-line"></span>
          </div>

          <div
            v-else
            v-memo="[group, idx, historyGroupMemoKey(group, idx)]"
            class="chat-transcript-message"
            :class="[
              `is-${variant}`,
              group.role,
              {
                'has-round-divider': shouldShowSessionRoundDivider(group, idx),
                'before-continuation': isStreamingContinuation && idx === groupedMessages.length - 1,
                'compact-handoff': group.items.some((item) => isCompactHandoffMessage(item.message)),
                'user-align-right': shouldRightAlignUserMessageGroup(group),
              },
            ]"
            :data-chat-message-id="group.items[0]?.id"
            :data-chat-message-role="group.role"
            :data-chat-message-group-role="group.role"
            :data-chat-message-group-start-id="group.items[0]?.id"
            :data-chat-message-group-end-id="group.items[group.items.length - 1]?.id"
          >
          <div class="chat-transcript-message-role" :class="`is-${variant}`">
            {{ messageGroupLabel(group) }}
          </div>

          <div class="chat-transcript-message-content" :class="`is-${variant}`">
            <template v-if="group.role === 'user'">
              <div
                v-for="item in group.items"
                v-show="shouldRenderItem(item)"
                :key="item.id"
                class="chat-transcript-item-stack"
                :class="[
                  `is-${variant}`,
                  { 'is-context-selected': isContextSelectedMessage(item.id) },
                ]"
                :data-scroll-anchor-id="item.id"
                :data-chat-message-id="item.id"
                :data-chat-message-role="item.message.role"
              >
                <div
                  v-if="enableIntentBadges && messageIntentBadges(item.message).length > 0"
                  class="chat-transcript-intent-row"
                >
                  <span
                    v-for="badge in messageIntentBadges(item.message)"
                    :key="badge.key"
                    class="chat-transcript-intent-badge"
                    :class="badge.kind"
                  >
                    <template v-if="badge.kind === 'skill'">
                      <span class="chat-transcript-intent-badge-mark">SKILL</span>
                      <span class="chat-transcript-intent-badge-divider"></span>
                      <span class="chat-transcript-intent-badge-text">{{ badge.label }}</span>
                    </template>
                    <template v-else>{{ badge.label }}</template>
                  </span>
                </div>

                <div
                  v-if="messageAssetRefs(item.message).length > 0"
                  class="chat-transcript-user-asset-refs"
                >
                  <AssetChip
                    v-for="assetRef in messageAssetRefs(item.message)"
                    :key="`${assetRef.kind}:${assetRef.path}`"
                    :path="assetRef.path"
                    :kind="assetRef.kind"
                    context-menu-mode="inherit"
                  />
                </div>

                <div
                  v-if="showUserImages && item.message.images && item.message.images.length > 0"
                  class="chat-transcript-user-images"
                >
                  <img
                    v-for="(_img, imgIdx) in item.message.images"
                    :key="imgIdx"
                    :src="imageDataUrl(item.message, imgIdx)"
                    class="chat-transcript-user-image-thumb"
                    @click.stop="openImage(imageDataUrl(item.message, imgIdx))"
                  />
                </div>

                <div
                  v-if="messageConsoleEntries(item.message).length > 0"
                  class="chat-transcript-user-console"
                >
                  <div class="chat-transcript-user-console-header">
                    <LucideIcon :icon="FileText" :size="14" />
                    <span>{{ t("chat.consoleRefs.defaultTitle") }}</span>
                    <span class="chat-transcript-user-console-count">{{ messageConsoleEntries(item.message).length }}</span>
                  </div>
                  <div class="chat-transcript-user-console-list">
                    <div
                      v-for="(entry, consoleIdx) in messageConsoleEntries(item.message)"
                      :key="`${item.id}:console:${consoleIdx}`"
                      class="chat-transcript-user-console-row"
                      :class="consoleEntryClass(entry)"
                    >
                      <span class="chat-transcript-user-console-level">{{ entry.level }}</span>
                      <span class="chat-transcript-user-console-title">{{ entry.title.replace(/^\[[^\]]+\]\s*/, "") }}</span>
                      <span class="chat-transcript-user-console-chars">{{ t("chat.consoleRefs.charCount", entry.chars) }}</span>
                    </div>
                  </div>
                </div>

                <div
                  v-if="messageLocalFileEntries(item.message).length > 0"
                  class="chat-transcript-user-local-files"
                >
                  <div
                    v-for="(entry, fileIdx) in messageLocalFileEntries(item.message)"
                    :key="localFileEntryKey(entry, fileIdx)"
                    class="chat-transcript-user-local-file"
                    :title="entry.path"
                  >
                    <MarkdownRenderer
                      class="chat-transcript-user-local-file-ref"
                      :content="localFileEntryMarkdown(entry)"
                      enable-file-refs
                    />
                    <span
                      v-if="entry.typeLabel"
                      class="chat-transcript-user-local-file-type"
                    >
                      {{ entry.typeLabel }}
                    </span>
                  </div>
                </div>

                <div v-if="userMessageDisplayContent(item.message)" class="chat-transcript-plain-text ui-select-text">
                  <template
                    v-for="(segment, segmentIdx) in userMessageDisplaySegments(item.message)"
                    :key="segmentIdx"
                  >
                    <AssetChip
                      v-if="segment.type === 'asset' || segment.type === 'knowledge'"
                      :path="segment.value"
                      :kind="segment.type === 'knowledge' ? 'knowledge' : undefined"
                      context-menu-mode="inherit"
                    />
                    <template v-else>{{ segment.value }}</template>
                  </template>
                </div>
              </div>
            </template>

            <div
              v-else
              class="chat-transcript-item-stack"
              :class="[
                `is-${variant}`,
                {
                  'tool-only': isToolOnlyMessageGroup(group),
                  'is-context-selected': isContextSelectedAssistantGroup(group),
                },
              ]"
              :data-scroll-anchor-id="group.items[0]?.id"
              :data-chat-message-id="group.items[0]?.id"
              data-chat-message-role="assistant"
            >
              <template
                v-for="segment in historyRenderSegmentsForGroup(group)"
                :key="segment.key"
              >
                <div
                  v-if="segment.type === 'thinking'"
                  class="chat-transcript-thinking-block"
                  data-render-part-kind="thinking"
                  data-render-part-scope="history"
                  :data-render-part-key="segment.key"
                  :data-chat-message-id="segment.itemId"
                  data-chat-message-role="assistant"
                >
                  <button
                    v-if="variant === 'session'"
                    type="button"
                    class="chat-transcript-thinking-header is-clickable"
                    @click="emit('openThinking', segment.content)"
                  >
                    <svg class="chat-transcript-thinking-chevron" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
                      <path d="M6 3l5 5-5 5V3z" />
                    </svg>
                    <span class="chat-transcript-thinking-title">
                      {{ formatThoughtSummary(segment.duration) }}
                    </span>
                  </button>

                  <div v-else class="chat-transcript-thinking-chip">
                    <span class="chat-transcript-thinking-title">
                      {{ formatThoughtSummary(segment.duration) }}
                    </span>
                  </div>
                </div>

                <div
                  v-else-if="segment.type === 'toolCalls'"
                  class="chat-transcript-tool-calls-group"
                  data-render-part-kind="toolCall"
                  data-render-part-scope="history"
                  :data-render-part-key="segment.key"
                  data-tool-layout-kind="group"
                  data-tool-layout-scope="history"
                  :data-tool-layout-key="segment.key"
                  :data-chat-message-id="segment.itemId"
                  data-chat-message-role="assistant"
                  :data-tool-layout-tool-call-ids="toolLayoutToolCallIds(segment.toolCalls)"
                  :data-tool-layout-statuses="toolLayoutStatuses(segment.toolCalls)"
                  :data-tool-layout-keep-expanded="String(shouldKeepToolSegmentExpanded(segment))"
                >
                  <ToolCallCollection
                    :tool-calls="segment.toolCalls"
                    :allow-collapse="!shouldKeepToolSegmentExpanded(segment)"
                    :collapse-enabled="!shouldKeepToolSegmentExpanded(segment)"
                    @viewport-anchor-start="emitToolViewportAnchorStart"
                    @viewport-anchor-end="emitToolViewportAnchorEnd"
                  >
                    <template #default="{ toolCall }">
                      <ToolCallBlock
                        :tool-call="toolCall"
                        :collapse-enabled="!shouldKeepToolSegmentExpanded(segment)"
                        @tool-viewport-anchor-start="emitToolViewportAnchorStart"
                        @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
                      />
                    </template>
                  </ToolCallCollection>
                </div>

                <MarkdownRenderer
                  v-else-if="segment.type === 'content'"
                  data-render-part-kind="text"
                  data-render-part-scope="history"
                  :data-render-part-key="segment.key"
                  :data-chat-message-id="segment.itemId"
                  data-chat-message-role="assistant"
                  :content="segment.content"
                  :unity-preview-state-scope="markdownUnityPreviewStateScope(segment)"
                  enable-file-refs
                  @open-image="openImage"
                />

                <KnowledgeProposalCard
                  v-else-if="segment.type === 'knowledgeProposal'"
                  data-render-part-kind="knowledgeProposal"
                  data-render-part-scope="history"
                  :data-render-part-key="segment.key"
                  :data-chat-message-id="segment.itemId"
                  data-chat-message-role="assistant"
                  :proposal="segment.message.knowledgeProposal!"
                  @apply="emit('applyKnowledgeProposal', $event)"
                  @ignore="emit('ignoreKnowledgeProposal', $event)"
                />
              </template>
            </div>
          </div>
          </div>
        </template>

        <div
          v-if="hasTransientAssistantMessage"
          class="chat-transcript-message assistant transient"
          :class="[
            `is-${variant}`,
            {
              continuation: isStreamingContinuation,
              'waiting-placeholder': isStandaloneWaitingPlaceholder,
              'compact-handoff': isCompacting,
              'has-live-thinking': hasVisibleActiveThinkingBlock,
              'has-tool-waiting-status': isToolWaitingStatusVisible,
            },
          ]"
          data-scroll-anchor-id="__transient__"
        >
          <div
            v-if="!isStreamingContinuation"
            class="chat-transcript-message-role"
            :class="`is-${variant}`"
          >
            {{ isCompacting ? handoffLabel : assistantLabel }}
          </div>

          <div class="chat-transcript-message-content" :class="`is-${variant}`">
            <div class="chat-transcript-item-stack" :class="`is-${variant}`">
              <template
                v-for="segment in transientRenderSegments"
                :key="segment.key"
              >
                <div
                  v-if="segment.type === 'thinking'"
                  class="chat-transcript-thinking-block"
                  data-render-part-kind="thinking"
                  data-render-part-scope="transient"
                  :data-render-part-key="segment.key"
                >
                  <button
                    v-if="variant === 'session'"
                    type="button"
                    class="chat-transcript-thinking-header"
                    :class="{ active: segment.active, 'is-clickable': true }"
                    @click="emit('openThinking', '')"
                  >
                    <svg class="chat-transcript-thinking-chevron" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
                      <path d="M6 3l5 5-5 5V3z" />
                    </svg>
                    <template v-if="segment.active">
                      <ChatWaitingIndicator :label="thinkingActiveLabel" />
                    </template>
                    <template v-else>
                      <span class="chat-transcript-thinking-title">{{ formatThoughtSummary(segment.duration) }}</span>
                    </template>
                  </button>

                  <div v-else class="chat-transcript-thinking-chip" :class="{ active: segment.active }">
                    <ChatWaitingIndicator v-if="segment.active" :label="thinkingActiveLabel" compact />
                    <span v-else class="chat-transcript-thinking-title">
                      {{ formatThoughtSummary(segment.duration) }}
                    </span>
                  </div>
                </div>

                <div
                  v-else-if="segment.type === 'toolCalls'"
                  class="chat-transcript-tool-calls-group"
                  data-render-part-kind="toolCall"
                  data-render-part-scope="transient"
                  :data-render-part-key="segment.key"
                  data-tool-layout-kind="group"
                  data-tool-layout-scope="transient"
                  :data-tool-layout-key="segment.key"
                  :data-tool-layout-tool-call-ids="toolLayoutToolCallIds(segment.toolCalls)"
                  :data-tool-layout-statuses="toolLayoutStatuses(segment.toolCalls)"
                  :data-tool-layout-allow-collapse="String(segment.allowCollapse)"
                  :data-tool-layout-collapse-enabled="String(segment.collapseEnabled)"
                  :data-tool-layout-animate-collapse-on-mount="String(segment.animateCollapseOnMount)"
                  :data-tool-layout-show-waiting="String(segment.showWaiting && isToolWaitingRowVisible)"
                  :data-tool-layout-waiting-status="String(segment.showWaiting && isToolWaitingStatusVisible)"
                >
                  <ToolCallCollection
                    :tool-calls="segment.toolCalls"
                    :allow-collapse="segment.allowCollapse"
                    :collapse-enabled="segment.collapseEnabled"
                    :animate-collapse-on-mount="segment.animateCollapseOnMount"
                    :show-waiting-status="segment.showWaiting && isToolWaitingStatusVisible"
                    :waiting-label="effectiveWaitingLabel"
                    @collapse-finished="onTransientToolCallsCollapseFinished"
                    @viewport-anchor-start="emitToolViewportAnchorStart"
                    @viewport-anchor-end="emitToolViewportAnchorEnd"
                  >
                    <template #default="{ toolCall }">
                      <ToolCallBlock
                        :tool-call="toolCall"
                        :collapse-enabled="segment.collapseEnabled"
                        @tool-viewport-anchor-start="emitToolViewportAnchorStart"
                        @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
                      />
                    </template>
                  </ToolCallCollection>
                  <div v-if="segment.showWaiting && isToolWaitingRowVisible" class="chat-transcript-tool-waiting-row">
                    <ChatWaitingIndicator :label="effectiveWaitingLabel" compact />
                  </div>
                </div>

                <div
                  v-else-if="segment.type === 'waiting'"
                  class="chat-transcript-thinking-block"
                  data-render-part-kind="waiting"
                  data-render-part-scope="transient"
                  :data-render-part-key="segment.key"
                >
                  <div class="chat-transcript-thinking-header active">
                    <ChatWaitingIndicator :label="segment.label" />
                  </div>
                </div>

                <MarkdownRenderer
                  v-else-if="segment.type === 'content'"
                  data-render-part-kind="text"
                  data-render-part-scope="transient"
                  :data-render-part-key="segment.key"
                  :content="segment.content"
                  :unity-preview-state-scope="markdownUnityPreviewStateScope(segment)"
                  cursor
                  enable-file-refs
                  @open-image="openImage"
                />
              </template>
            </div>
          </div>
        </div>

        <div
          v-if="showSessionFooter"
          class="chat-transcript-footer"
          :class="`is-${variant}`"
        >
          <slot name="footer" />
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.chat-transcript-scroll {
  flex: 1;
  width: 100%;
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
}

.chat-transcript-content {
  width: 100%;
  min-width: 0;
  min-height: 100%;
}

.chat-transcript-scroll.is-session {
  --chat-transcript-session-segment-gap: 10px;
  --chat-transcript-session-bottom-gap: 40px;
  padding: 24px 0 0;
  background: var(--msg-assistant-bg);
  overflow-anchor: none;
  contain: layout paint;
}

.chat-transcript-scroll.is-session > .chat-transcript-content {
  box-sizing: border-box;
  padding-bottom: var(--chat-transcript-session-bottom-gap);
}

.chat-transcript-scroll.is-embedded {
  padding: 10px 0 14px;
}

.chat-transcript-empty.is-session {
  min-height: 100%;
  display: flex;
}

.chat-transcript-empty.is-embedded {
  padding: 18px 14px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.chat-transcript-empty-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.chat-transcript-empty-hint {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.6;
}

.chat-transcript-message.is-session {
  padding: 12px 48px;
  max-width: 100%;
  position: relative;
  contain: layout paint;
}

@supports (content-visibility: auto) {
  .chat-transcript-message.is-session {
    content-visibility: auto;
    contain-intrinsic-size: auto 180px;
  }

  .chat-transcript-scroll.is-session.is-session-restore-stabilizing .chat-transcript-message.is-session {
    content-visibility: visible;
  }

  .chat-transcript-message.is-session.assistant.transient.has-live-thinking,
  .chat-transcript-message.is-session.assistant.transient.has-tool-waiting-status {
    content-visibility: visible;
  }
}

.chat-transcript-message.is-session.assistant {
  background: var(--msg-assistant-bg);
}

.chat-transcript-message.is-session.assistant.transient.continuation {
  border-top: none;
}

.chat-transcript-message.is-session.user {
  background: var(--msg-assistant-bg);
}

.chat-transcript-message.is-session.has-round-divider {
  border-top: 2px solid var(--msg-divider);
}

.chat-transcript-message.is-session.assistant.transient.waiting-placeholder {
  background: var(--msg-assistant-bg);
  border-top: none;
  border-bottom: none;
}

.chat-transcript-message.is-session.assistant.transient.has-live-thinking,
.chat-transcript-message.is-session.assistant.transient.has-tool-waiting-status {
  contain: layout;
}

.chat-transcript-message.is-session.compact-handoff {
  border-top: 1px solid color-mix(in srgb, var(--accent-color) 18%, transparent);
  background:
    linear-gradient(
      180deg,
      color-mix(in srgb, var(--accent-color) 5%, var(--msg-assistant-bg)),
      var(--msg-assistant-bg)
    );
}

.chat-transcript-message.is-session.user + .chat-transcript-message.is-session.assistant {
  border-top: none;
}

.chat-transcript-message.is-session.compact-handoff + .chat-transcript-message.is-session.user {
  border-top-color: color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.chat-transcript-compact-marker {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1;
  user-select: none;
}

.chat-transcript-compact-marker.is-session {
  padding: 10px 48px;
  background: var(--msg-assistant-bg);
}

.chat-transcript-compact-marker.is-embedded {
  padding: 8px 14px;
}

.chat-transcript-compact-marker-line {
  height: 1px;
  flex: 1;
  min-width: 24px;
  background: var(--border-color);
}

.chat-transcript-compact-marker-label {
  flex: 0 0 auto;
  white-space: nowrap;
}

.chat-transcript-message.is-session.before-continuation {
  padding-bottom: 0;
}

.chat-transcript-message.is-session.continuation {
  padding-top: var(--chat-transcript-session-segment-gap);
}

.chat-transcript-message.is-embedded {
  padding: 10px 14px;
}

.chat-transcript-message.is-embedded.assistant {
  background: color-mix(in srgb, var(--panel-bg) 88%, transparent);
}

.chat-transcript-message.is-embedded.transient {
  border-top: 1px solid color-mix(in srgb, var(--accent-color) 14%, transparent);
}

.chat-transcript-message.is-embedded.transient.continuation {
  border-top: none;
}

.chat-transcript-footer.is-session {
  display: flex;
  justify-content: flex-start;
  align-items: center;
  padding: 10px 48px 12px;
  background: var(--msg-assistant-bg);
}

.chat-transcript-message-role {
  color: var(--text-secondary);
  text-transform: uppercase;
}

.chat-transcript-message-role.is-session {
  margin-bottom: 4px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.5px;
}

.chat-transcript-message.is-session.compact-handoff .chat-transcript-message-role {
  color: var(--accent-color);
}

.chat-transcript-message.is-session.user .chat-transcript-message-role.is-session {
  color: var(--msg-user-role);
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-role.is-session {
  text-align: right;
}

.chat-transcript-message-role.is-embedded {
  margin-bottom: 5px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
}

.chat-transcript-message-content.is-session {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
  font-size: 14px;
  line-height: 1.6;
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-content.is-session {
  align-items: flex-end;
}

.chat-transcript-message-content.is-embedded {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
  font-size: 13px;
  color: var(--text-color);
  line-height: 1.62;
}

.chat-transcript-item-stack {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.chat-transcript-item-stack.is-session {
  gap: var(--chat-transcript-session-segment-gap);
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-item-stack.is-session {
  width: fit-content;
  max-width: min(100%, 78ch);
  align-items: flex-end;
}

.chat-transcript-item-stack.is-embedded {
  gap: 9px;
}

.chat-transcript-item-stack > [data-render-part-kind] {
  position: relative;
  z-index: 1;
}

.chat-transcript-item-stack.is-context-selected {
  border-radius: 8px;
  outline: 1px solid color-mix(in srgb, var(--accent-color) 42%, var(--border-color));
  outline-offset: 4px;
  background: color-mix(in srgb, var(--accent-soft) 36%, transparent);
  transition: background 120ms ease, outline-color 120ms ease;
}

.chat-transcript-intent-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.chat-transcript-intent-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.02em;
  background: var(--hover-bg);
  color: var(--text-secondary);
}

.chat-transcript-intent-badge.plan {
  color: #1d4ed8;
  background: color-mix(in srgb, #3b82f6 14%, transparent);
}

.chat-transcript-intent-badge.skill {
  display: inline-grid;
  grid-template-columns: auto 1px minmax(0, auto);
  align-items: stretch;
  gap: 0;
  min-height: 28px;
  padding: 0;
  overflow: hidden;
  color: var(--text-color);
  border-color: color-mix(in srgb, var(--accent-color) 26%, var(--border-color));
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--msg-user-bg) 26%);
  line-height: 1;
}

.chat-transcript-intent-badge-mark {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 42px;
  padding: 0 8px;
  color: var(--accent-color);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.04em;
  line-height: 1;
}

.chat-transcript-intent-badge-divider {
  align-self: stretch;
  width: 1px;
  background: color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
}

.chat-transcript-intent-badge-text {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  max-width: 180px;
  padding: 0 8px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 600;
  line-height: 1;
}

.chat-transcript-plain-text {
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-color);
}

.chat-transcript-message.is-session.user .chat-transcript-plain-text {
  align-self: flex-start;
  width: fit-content;
  max-width: min(100%, 78ch);
  padding: 6px 12px;
  border-radius: 8px;
  background: var(--msg-user-bg);
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-plain-text {
  align-self: flex-end;
  text-align: left;
}

.chat-transcript-user-asset-refs {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  max-width: min(100%, 78ch);
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-user-asset-refs {
  justify-content: flex-end;
}

.chat-transcript-user-asset-refs :deep(.asset-chip) {
  max-width: min(320px, 100%);
  background: color-mix(in srgb, var(--panel-bg) 68%, var(--msg-user-bg) 32%);
  border-color: color-mix(in srgb, var(--border-color) 88%, transparent);
}

.chat-transcript-user-asset-refs :deep(.asset-chip:hover) {
  background: color-mix(in srgb, var(--hover-bg) 82%, var(--panel-bg) 18%);
  border-color: color-mix(in srgb, var(--accent-color) 35%, var(--border-color));
}

.chat-transcript-user-images {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.chat-transcript-user-image-thumb {
  max-width: 240px;
  max-height: 180px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  object-fit: contain;
  cursor: pointer;
}

.chat-transcript-user-console {
  width: min(520px, 100%);
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 68%, var(--msg-user-bg) 32%);
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-user-console {
  align-self: flex-end;
}

.chat-transcript-user-console-header {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  padding: 5px 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.chat-transcript-user-console-header :deep(svg) {
  color: var(--text-secondary);
}

.chat-transcript-user-console-count {
  margin-left: auto;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 500;
}

.chat-transcript-user-console-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 4px;
}

.chat-transcript-user-console-row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 8px;
  min-height: 26px;
  padding: 4px 6px;
  border-radius: 6px;
  color: var(--text-color);
  font-size: 12px;
}

.chat-transcript-user-console-row:hover {
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
}

.chat-transcript-user-console-level {
  min-width: 46px;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
}

.chat-transcript-user-console-row.level-error .chat-transcript-user-console-level {
  color: var(--status-error-fg, var(--text-color));
}

.chat-transcript-user-console-row.level-warning .chat-transcript-user-console-level {
  color: var(--status-warn-fg, var(--text-color));
}

.chat-transcript-user-console-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chat-transcript-user-console-chars {
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
}

.chat-transcript-user-local-files {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  max-width: min(100%, 78ch);
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-user-local-files {
  justify-content: flex-end;
}

.chat-transcript-user-local-file {
  min-width: 0;
  max-width: min(360px, 100%);
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.chat-transcript-user-local-file :deep(.markdown-body) {
  min-width: 0;
  display: inline-flex;
  line-height: 22px;
}

.chat-transcript-user-local-file :deep(.markdown-body p) {
  min-width: 0;
  display: inline-flex;
  margin: 0;
}

.chat-transcript-user-local-file :deep(.md-file-ref) {
  max-width: 100%;
  background: color-mix(in srgb, var(--panel-bg) 68%, var(--msg-user-bg) 32%);
  border-color: color-mix(in srgb, var(--border-color) 88%, transparent);
}

.chat-transcript-user-local-file :deep(.md-file-ref:hover) {
  background: color-mix(in srgb, var(--hover-bg) 82%, var(--panel-bg) 18%);
  border-color: var(--border-strong);
}

.chat-transcript-user-local-file :deep(.md-ref-label) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chat-transcript-user-local-file-type {
  flex: 0 0 auto;
  max-width: 72px;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
}

.chat-transcript-thinking-block {
  position: relative;
  z-index: 0;
  display: flex;
  align-items: flex-start;
  min-width: 0;
  min-height: 28px;
}

.chat-transcript-thinking-block[data-render-part-scope="transient"] {
  z-index: 3;
}

.chat-transcript-thinking-header {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  min-height: 28px;
  padding: 4px 10px 4px 6px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 13px;
  line-height: 1.35;
  text-align: left;
  overflow: visible;
}

.chat-transcript-thinking-header.is-clickable {
  cursor: pointer;
  transition: background 0.15s;
}

.chat-transcript-thinking-header.is-clickable:hover {
  background: var(--hover-bg);
}

.chat-transcript-thinking-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 24px;
  padding: 4px 8px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--hover-bg) 78%, transparent);
  color: var(--text-secondary);
  line-height: 1.35;
}

.chat-transcript-thinking-chip.active {
  color: var(--text-color);
}

.chat-transcript-thinking-chevron {
  flex-shrink: 0;
  transition: transform 0.2s ease;
  opacity: 0.5;
}

.chat-transcript-thinking-title {
  font-weight: 500;
  white-space: nowrap;
}

.chat-transcript-tool-calls-group {
  position: relative;
  z-index: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-left: -4px;
}

.chat-transcript-message-content > .chat-transcript-item-stack.tool-only:first-child {
  margin-top: 4px;
}

.chat-transcript-tool-waiting-row {
  display: inline-flex;
  align-items: center;
  align-self: flex-start;
  gap: 6px;
  min-height: 22px;
  padding: 1px 4px;
  color: var(--text-secondary);
  font-size: 13px;
}
</style>
