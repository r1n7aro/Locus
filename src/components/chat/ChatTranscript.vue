<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, useSlots, watch } from "vue";
import { FileText } from "lucide";
import type { AssetRefAttachment, AssistantRenderPart, ChatMessage, ToolCallDisplay, ToolCallInfo, UserIntentMeta } from "../../types";
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
import {
  assertCanonicalRenderParts,
  compareAssistantRenderParts,
  synthesizeLegacyRenderParts,
} from "../../composables/assistantRenderParts";
import { useDisplaySettings } from "../../composables/useDisplaySettings";
import { parseChatAssetRefs } from "../../composables/chatAssetRefs";
import {
  displayUserMessageContent,
  userMessageConsoleEntries,
  type UserConsoleEntryDisplay,
} from "../../composables/chatUserMessageDisplay";
import { logToolCollapseTrace, previewTraceText } from "../../services/toolCollapseTrace";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import ToolCallCollection from "../ToolCallCollection.vue";
import ToolCallBlock from "../ToolCallBlock.vue";
import KnowledgeProposalCard from "./KnowledgeProposalCard.vue";
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
});

const emit = defineEmits<{
  (e: "applyKnowledgeProposal", proposalId: string): void;
  (e: "ignoreKnowledgeProposal", proposalId: string): void;
  (e: "openThinking", content: string): void;
  (e: "openImage", src: string): void;
  (e: "scroll", event: Event): void;
  (e: "contentClick", event: MouseEvent): void;
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
      label: `SKILL: ${skill.name}`,
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

function consoleEntryClass(entry: UserConsoleEntryDisplay) {
  return `level-${entry.level.toLowerCase()}`;
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

const visibleMessages = computed(() =>
  props.messages.filter((msg) => {
    const status = msg.knowledgeProposal?.status;
    return status !== "stale" && status !== "invalidated";
  }),
);

const toolCallHandoff = ref<ToolCallHandoffState | null>(null);
const toolCallHandoffQuiet = ref(false);
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
  return {
    id: toolCall.id,
    name: toolCall.name,
    arguments: toolCall.arguments,
    status: toolCall.status,
    order: toolCall.order,
    output: toolCall.output,
    nestedToolCalls: toolCall.nestedToolCalls?.map((nestedToolCall) => cloneToolCallDisplay(nestedToolCall)),
  };
}

function cloneToolCallDisplays(toolCalls: ToolCallDisplay[]) {
  return toolCalls.map((toolCall) => cloneToolCallDisplay(toolCall));
}

function traceToolCollapse(event: string, detail?: Record<string, unknown>) {
  logToolCollapseTrace(`transcript:${props.variant}`, event, detail);
}

function clearToolCallHandoff(reason = "clear") {
  const handoff = toolCallHandoff.value;
  if (handoff) {
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

  clearToolCallHandoffTimers();
  setToolCallHandoffQuiet(false);
  toolCallHandoff.value = {
    renderKey: `tool-handoff-${++toolCallHandoffSequence}:${toolCalls.map((toolCall) => toolCall.id).join(",")}`,
    createdAt: Date.now(),
    toolCalls,
    toolCallIds: collectToolCallDisplayIds(toolCalls),
    toolCallMatchState: collectToolCallDisplayMatchState(toolCalls),
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

watch(
  () => props.activeToolCalls,
  (activeToolCalls, previousToolCalls) => {
    if (activeToolCalls.length === 0 && previousToolCalls && previousToolCalls.length > 0) {
      const previousMatchState = collectToolCallDisplayMatchState(previousToolCalls);
      traceToolCollapse("activeToolCallsCleared", {
        previousCount: previousToolCalls.length,
        previousIds: previousToolCalls.map((toolCall) => toolCall.id),
        streamingTextLen: props.streamingText.length,
        isStreaming: props.isStreaming,
      });
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
  (activeToolCallCount) => {
    if (activeToolCallCount > 0 && toolCallHandoff.value) {
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
    if (messages === previous || !toolCallHandoff.value) return;
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
    traceToolCollapse("isStreamingChanged", {
      previous: previousStreaming,
      next: isStreaming,
      hasHandoff: !!handoff,
      collapseArmed: handoff?.collapseArmed ?? false,
      collapseFinished: handoff?.collapseFinished ?? false,
    });
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

function toolCallsForRenderItem(item: Pick<MessageRenderItem, "message" | "displayToolCalls">) {
  return buildMessageToolCalls(
    {
      toolCalls: resolveToolCallInfosForRender({
        messageToolCalls: toolCallInfosForMessage(item.message),
        displayToolCalls: item.displayToolCalls,
      }),
    },
    toolOutputMap.value,
  );
}

function toolCallsFromInfos(toolCalls: ToolCallInfo[] | undefined) {
  return buildMessageToolCalls({ toolCalls }, toolOutputMap.value);
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

function isToolOnlyRenderItem(item: MessageRenderItem) {
  if (item.message.role !== "assistant" || item.message.knowledgeProposal) return false;
  const parts = renderPartsForMessage(item);
  if (parts.length > 0) {
    return parts.every((part) => part.kind === "toolCall");
  }

  const hasAttachedKnowledgeProposals = item.attachedKnowledgeProposals.length > 0;
  const hasToolCalls = toolCallsForRenderItem(item).length > 0;

  return !item.message.content.trim() && !shouldRenderHistoryThinkingBlock(item) && !hasAttachedKnowledgeProposals && hasToolCalls;
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
  if (!toolCallHandoff.value) return emptyPromotedHistoryToolCalls();

  const lastGroup = baseGroupedMessages.value[baseGroupedMessages.value.length - 1];
  if (!lastGroup || lastGroup.role !== "assistant") {
    return emptyPromotedHistoryToolCalls();
  }

  const collectedItemIds = new Set<string>();
  const collectedBatches: ToolCallDisplay[][] = [];

  for (let index = lastGroup.items.length - 1; index >= 0; index -= 1) {
    const item = lastGroup.items[index];
    if (!item || !isToolOnlyRenderItem(item)) break;
    const itemToolCalls = toolCallsForRenderItem(item);
    if (itemToolCalls.length === 0) break;
    collectedItemIds.add(item.id);
    collectedBatches.unshift(cloneToolCallDisplays(itemToolCalls));
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
  if (!toolCallHandoff.value || promotableHistoryToolCalls.value.toolCalls.length === 0) {
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

const historyHiddenToolCallMatchState = computed<ToolCallMatchState>(() => {
  if (!toolCallHandoff.value?.collapseArmed) {
    return activeToolCallMatchState.value;
  }
  return mergeToolCallMatchStates(
    activeToolCallMatchState.value,
    promotableHistoryToolCalls.value.toolCallMatchState,
  );
});

const groupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(historyHiddenToolCallMatchState.value));

const transientToolCalls = computed(() => {
  if (!toolCallHandoff.value?.collapseArmed || promotableHistoryToolCalls.value.toolCalls.length === 0) {
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
  traceToolCollapse("promotableHistoryToolCallsChanged", {
    previousItemCount: previousState?.itemIds.size ?? 0,
    nextItemCount: nextState.itemIds.size,
    nextToolCallCount: nextState.toolCalls.length,
    nextToolCallIds: Array.from(nextState.toolCallIds),
    hasHandoff: !!toolCallHandoff.value,
  });
});

watch(transientToolCallsCollapseEnabled, (enabled, previousEnabled) => {
  traceToolCollapse("transientToolCallsCollapseEnabledChanged", {
    previous: previousEnabled,
    next: enabled,
    transientToolCallCount: transientToolCalls.value.length,
    hasHandoff: !!toolCallHandoff.value,
  });
});

function onTransientToolCallsCollapseFinished() {
  if (!toolCallHandoff.value?.collapseArmed) return;
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

const canonicalLiveRenderParts = computed(() => {
  if (props.liveRenderParts.length > 0) {
    assertCanonicalRenderParts(props.liveRenderParts, "live");
    return [...props.liveRenderParts].sort(compareAssistantRenderParts);
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
const isStandaloneWaitingPlaceholder = computed(() => isWaitingForResponse.value && !hasTransientToolCalls.value);
const hasStandaloneCompactingPlaceholder = computed(() => props.isCompacting);
const hasTransientAssistantMessage = computed(
  () =>
    hasStreamingContent.value
    || hasToolCallHandoff.value
    || hasVisibleTransientThinkingBlock.value
    || isWaitingForResponse.value
    || hasStandaloneCompactingPlaceholder.value,
);

watch(
  () => `${Number(isStandaloneWaitingPlaceholder.value)}:${Number(isToolWaitingForResponse.value)}:${transientToolCalls.value.length}`,
  (next, previous) => {
    traceToolCollapse("waitingLayoutStateChanged", {
      previous,
      next,
      standaloneWaiting: isStandaloneWaitingPlaceholder.value,
      toolWaiting: isToolWaitingForResponse.value,
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

const nonCollapsibleToolItemIds = computed(() => {
  return new Set(pendingContinuationToolItemIds.value);
});

function shouldKeepToolItemExpanded(itemId: string) {
  return nonCollapsibleToolItemIds.value.has(itemId);
}

type HistoryRenderSegment =
  | { type: "thinking"; key: string; part: AssistantRenderPart; content: string; duration?: number }
  | { type: "toolCalls"; key: string; part: Extract<AssistantRenderPart, { kind: "toolCall" }>; itemId: string; itemIds: string[]; toolCalls: ToolCallDisplay[] }
  | { type: "content"; key: string; part: AssistantRenderPart; content: string }
  | { type: "knowledgeProposal"; key: string; part: AssistantRenderPart; message: ChatMessage };

type TransientRenderSegment =
  | { type: "thinking"; key: string; part: AssistantRenderPart; active: boolean; duration?: number }
  | { type: "toolCalls"; key: string; part: Extract<AssistantRenderPart, { kind: "toolCall" }>; toolCalls: ToolCallDisplay[]; showWaiting: boolean; animateCollapseOnMount: boolean }
  | { type: "waiting"; key: string; label: string }
  | { type: "content"; key: string; part: AssistantRenderPart; content: string };

function renderPartsForMessage(item: MessageRenderItem): AssistantRenderPart[] {
  const hasToolFilter = hasExplicitDisplayToolCalls(item) || !!toolCallInfosForMessage(item.message);
  const messageToolCalls = resolveToolCallInfosForRender({
    messageToolCalls: toolCallInfosForMessage(item.message),
    displayToolCalls: item.displayToolCalls,
  });

  let parts: AssistantRenderPart[];
  if (item.message.renderParts?.length) {
    assertCanonicalRenderParts(item.message.renderParts, `message:${item.message.id}`);
    const visibleToolIds = new Set((messageToolCalls ?? []).map((toolCall) => toolCall.id));
    parts = [...item.message.renderParts]
      .filter((part) => part.kind !== "thinking" || !shouldHideThinkingBlocks())
      .filter((part) => part.kind !== "toolCall" || !hasToolFilter || visibleToolIds.has(part.toolCall.id))
      .sort(compareAssistantRenderParts);
  } else {
    parts = synthesizeLegacyRenderParts(item.message, {
      toolCalls: messageToolCalls,
      beforeContentToolCalls: item.displayToolCallsBeforeContent,
      afterContentToolCalls: item.displayToolCallsAfterContent,
      knowledgeProposals: item.attachedKnowledgeProposals,
    }).filter((part) => part.kind !== "thinking" || !shouldHideThinkingBlocks());
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
        content: part.content,
        duration: part.duration,
      });
    } else if (part.kind === "text") {
      flushPendingTools();
      segments.push({
        type: "content",
        key: `${item.id}:${part.id}`,
        part,
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
  return segment.itemIds.some((itemId) => shouldKeepToolItemExpanded(itemId));
}

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
      const content = props.liveRenderParts.length <= 1 && props.streamingText ? props.streamingText : part.content;
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

  const hasRenderedToolSegment = segments.some((segment) => segment.type === "toolCalls");
  if (!hasRenderedToolSegment && transientToolCalls.value.length > 0) {
    const handoffPart = transientToolHandoffPart(transientToolCalls.value);
    if (handoffPart) {
      segments.unshift({
        type: "toolCalls",
        key: transientToolSegmentKey(transientToolCalls.value),
        part: handoffPart,
        toolCalls: transientToolCalls.value,
        showWaiting: false,
        animateCollapseOnMount: !!toolCallHandoff.value?.collapseArmed,
      });
    }
  }

  const toolSegments = segments.filter((segment) => segment.type === "toolCalls");
  const lastToolSegment = toolSegments[toolSegments.length - 1];
  if (lastToolSegment) {
    lastToolSegment.showWaiting = true;
  }

  if (hasStandaloneCompactingPlaceholder.value || isStandaloneWaitingPlaceholder.value) {
    segments.push({
      type: "waiting",
      key: hasStandaloneCompactingPlaceholder.value ? "transient:compacting" : "transient:waiting",
      label: hasStandaloneCompactingPlaceholder.value ? props.compactingLabel : props.waitingLabel,
    });
  }

  return segments;
});

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

function emitContentClick(event: MouseEvent) {
  emit("contentClick", event);
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
    @click="emitContentClick"
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
                :class="`is-${variant}`"
                :data-scroll-anchor-id="item.id"
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
                    {{ badge.label }}
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

                <div v-if="userMessageDisplayContent(item.message)" class="chat-transcript-plain-text ui-select-text">
                  <template
                    v-for="(segment, segmentIdx) in userMessageDisplaySegments(item.message)"
                    :key="segmentIdx"
                  >
                    <AssetChip
                      v-if="segment.type === 'asset' || segment.type === 'knowledge'"
                      :path="segment.value"
                      :kind="segment.type === 'knowledge' ? 'knowledge' : undefined"
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
                },
              ]"
              :data-scroll-anchor-id="group.items[0]?.id"
            >
              <template
                v-for="segment in historyRenderSegmentsForGroup(group)"
                :key="segment.key"
              >
                <div
                  v-if="segment.type === 'thinking'"
                  class="chat-transcript-thinking-block"
                  data-render-part-kind="thinking"
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
                  :content="segment.content"
                  enable-file-refs
                />

                <KnowledgeProposalCard
                  v-else-if="segment.type === 'knowledgeProposal'"
                  data-render-part-kind="knowledgeProposal"
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
            <template
              v-for="segment in transientRenderSegments"
              :key="segment.key"
            >
              <div
                v-if="segment.type === 'thinking'"
                class="chat-transcript-thinking-block"
                data-render-part-kind="thinking"
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
                    <span class="chat-transcript-thinking-spinner" />
                    <span class="chat-transcript-thinking-title">{{ thinkingActiveLabel }}</span>
                  </template>
                  <template v-else>
                    <span class="chat-transcript-thinking-title">{{ formatThoughtSummary(segment.duration) }}</span>
                  </template>
                </button>

                <div v-else class="chat-transcript-thinking-chip" :class="{ active: segment.active }">
                  <span v-if="segment.active" class="chat-transcript-thinking-spinner compact" />
                  <span class="chat-transcript-thinking-title">
                    {{ segment.active ? thinkingActiveLabel : formatThoughtSummary(segment.duration) }}
                  </span>
                </div>
              </div>

              <div
                v-else-if="segment.type === 'toolCalls'"
                class="chat-transcript-tool-calls-group"
                data-render-part-kind="toolCall"
              >
                <ToolCallCollection
                  :tool-calls="segment.toolCalls"
                  :allow-collapse="transientToolCallsAllowCollapse"
                  :collapse-enabled="transientToolCallsCollapseEnabled"
                  :animate-collapse-on-mount="segment.animateCollapseOnMount"
                  @collapse-finished="onTransientToolCallsCollapseFinished"
                  @viewport-anchor-start="emitToolViewportAnchorStart"
                  @viewport-anchor-end="emitToolViewportAnchorEnd"
                >
                  <template #default="{ toolCall }">
                    <ToolCallBlock
                      :tool-call="toolCall"
                      :collapse-enabled="transientToolCallsCollapseEnabled"
                      @tool-viewport-anchor-start="emitToolViewportAnchorStart"
                      @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
                    />
                  </template>
                </ToolCallCollection>
                <div v-if="segment.showWaiting && isToolWaitingForResponse" class="chat-transcript-tool-waiting-row">
                  <span class="chat-transcript-thinking-spinner compact" />
                  <span class="chat-transcript-thinking-title">{{ waitingLabel }}</span>
                </div>
              </div>

              <div v-else-if="segment.type === 'waiting'" class="chat-transcript-thinking-block">
                <div class="chat-transcript-thinking-header active">
                  <span class="chat-transcript-thinking-spinner" />
                  <span class="chat-transcript-thinking-title">{{ segment.label }}</span>
                </div>
              </div>

              <MarkdownRenderer
                v-else-if="segment.type === 'content'"
                data-render-part-kind="text"
                :content="segment.content"
                cursor
                enable-file-refs
              />
            </template>
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
  padding: 24px 0;
  background: var(--msg-assistant-bg);
  overflow-anchor: none;
  contain: layout paint;
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

  .chat-transcript-message.is-session.assistant.transient.waiting-placeholder {
    content-visibility: visible;
    contain-intrinsic-size: auto 0;
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
  padding-top: 8px;
  padding-bottom: 0;
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
  padding-top: 6px;
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
  gap: 10px;
}

.chat-transcript-message.is-session.user.user-align-right .chat-transcript-item-stack.is-session {
  width: fit-content;
  max-width: min(100%, 78ch);
  align-items: flex-end;
}

.chat-transcript-item-stack.is-embedded {
  gap: 9px;
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
  border-radius: var(--radius-badge);
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
  color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-color) 14%, transparent);
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

.chat-transcript-thinking-header {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px 4px 6px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 13px;
  text-align: left;
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
}

.chat-transcript-thinking-chip.active {
  color: var(--text-color);
}

.chat-transcript-thinking-chevron {
  flex-shrink: 0;
  transition: transform 0.2s ease;
  opacity: 0.5;
}

.chat-transcript-thinking-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid var(--border-color);
  border-top-color: var(--text-secondary);
  border-radius: 50%;
  animation: chat-transcript-thinking-spin 0.8s linear infinite;
  flex-shrink: 0;
}

.chat-transcript-thinking-spinner.compact {
  width: 12px;
  height: 12px;
}

@keyframes chat-transcript-thinking-spin {
  to {
    transform: rotate(360deg);
  }
}

.chat-transcript-thinking-title {
  font-weight: 500;
  white-space: nowrap;
}

.chat-transcript-thinking-header.active .chat-transcript-thinking-title {
  background: linear-gradient(90deg, var(--text-secondary) 0%, var(--text-color) 50%, var(--text-secondary) 100%);
  background-size: 200% 100%;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
  animation: chat-transcript-thinking-shimmer 2s ease-in-out infinite;
}

@keyframes chat-transcript-thinking-shimmer {
  0% {
    background-position: 100% 0;
  }

  100% {
    background-position: -100% 0;
  }
}

.chat-transcript-tool-calls-group {
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
