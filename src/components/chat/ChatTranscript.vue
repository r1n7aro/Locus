<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, useSlots, watch } from "vue";
import type { ChatMessage, ToolCallDisplay, ToolCallInfo, UserIntentMeta } from "../../types";
import {
  collectPendingContinuationToolItemIds,
  shouldShowAssistantContinuation,
  shouldShowWaitingPlaceholder,
} from "../../composables/chatViewStability";
import {
  buildMessageToolCalls,
  cloneToolCallMatchState,
  collectToolCallDisplayIds,
  collectToolCallDisplayMatchState,
  filterToolCallsByConsumableMatchState,
  getToolCallInfoFingerprint,
  mergeSequentialAssistantToolCalls,
  mergeToolCallDisplaysWithoutDuplicates,
  mergeToolCallMatchStates,
  resolveToolCallInfosForRender,
  summarizeToolCallBatch,
} from "../../composables/toolCallBatches";
import type { ToolCallMatchState } from "../../composables/toolCallBatches";
import { useDisplaySettings } from "../../composables/useDisplaySettings";
import { parseChatAssetRefs } from "../../composables/chatAssetRefs";
import { displayUserMessageContent } from "../../composables/chatUserMessageDisplay";
import { logToolCollapseTrace, previewTraceText } from "../../services/toolCollapseTrace";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import ToolCallCollection from "../ToolCallCollection.vue";
import ToolCallBlock from "../ToolCallBlock.vue";
import KnowledgeProposalCard from "./KnowledgeProposalCard.vue";
import AssetChip from "../AssetChip.vue";

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
  isThinking: boolean;
  hasThinking?: boolean;
  thinkingText?: string;
  thinkingOrder?: number;
  thinkingDuration?: number;
  activeToolCalls: ToolCallDisplay[];
  variant?: TranscriptVariant;
  emptyTitle?: string;
  emptyHint?: string;
  userLabel?: string;
  assistantLabel?: string;
  handoffLabel?: string;
  waitingLabel?: string;
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
  thinkingText: "",
  thinkingOrder: 0,
  thinkingDuration: 0,
  emptyTitle: "",
  emptyHint: "",
  userLabel: "User",
  assistantLabel: "Locus",
  handoffLabel: "Handoff",
  waitingLabel: "Waiting for response…",
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
const shouldArmToolCallHandoffCollapse = computed(
  () => hasVisibleStreamingText.value || !props.isStreaming,
);

function scheduleToolCallHandoffCollapse() {
  const handoff = toolCallHandoff.value;
  if (!handoff || handoff.collapseArmed) return;

  traceToolCollapse("scheduleToolCallHandoffCollapse", {
    renderKey: handoff.renderKey,
    createdAt: handoff.createdAt,
    toolCallCount: handoff.toolCalls.length,
    visibleStreamingText: hasVisibleStreamingText.value,
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

watch(
  () => props.activeToolCalls,
  (activeToolCalls, previousToolCalls) => {
    if (activeToolCalls.length === 0 && previousToolCalls && previousToolCalls.length > 0) {
      traceToolCollapse("activeToolCallsCleared", {
        previousCount: previousToolCalls.length,
        previousIds: previousToolCalls.map((toolCall) => toolCall.id),
        streamingTextLen: props.streamingText.length,
        isStreaming: props.isStreaming,
      });
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
    return collectToolCallDisplayMatchState(props.activeToolCalls);
  }
  return toolCallHandoff.value?.toolCallMatchState ?? {
    ids: new Set<string>(),
    fingerprintCounts: new Map<string, number>(),
    idFingerprints: new Map<string, string>(),
  };
});

function toolCallsForRenderItem(item: Pick<MessageRenderItem, "message" | "displayToolCalls">) {
  return buildMessageToolCalls(
    {
      toolCalls: resolveToolCallInfosForRender({
        messageToolCalls: item.message.toolCalls,
        displayToolCalls: item.displayToolCalls,
      }),
    },
    toolOutputMap.value,
  );
}

function toolCallsFromInfos(toolCalls: ToolCallInfo[] | undefined) {
  return buildMessageToolCalls({ toolCalls }, toolOutputMap.value);
}

function toolCallsBeforeContentForRenderItem(item: MessageRenderItem) {
  if (Object.prototype.hasOwnProperty.call(item, "displayToolCallsBeforeContent")) {
    return toolCallsFromInfos(item.displayToolCallsBeforeContent);
  }
  return item.message.content.trim().length > 0 ? [] : toolCallsForRenderItem(item);
}

function toolCallsAfterContentForRenderItem(item: MessageRenderItem) {
  if (Object.prototype.hasOwnProperty.call(item, "displayToolCallsAfterContent")) {
    return toolCallsFromInfos(item.displayToolCallsAfterContent);
  }
  return item.message.content.trim().length > 0 ? toolCallsForRenderItem(item) : [];
}

function shouldHideThinkingBlocks() {
  return displaySettings.hideThinkingBlocks !== false;
}

function shouldRenderHistoryThinkingBlock(item: Pick<MessageRenderItem, "message">) {
  return !shouldHideThinkingBlocks() && !!item.message.thinkingContent?.trim();
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
    item.message.content
    || shouldRenderHistoryThinkingBlock(item)
    || toolCallsForRenderItem(item).length > 0
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
    if (
      !item
      || item.hidden
      || item.message.knowledgeProposal
      || item.message.role !== "assistant"
      || !item.message.toolCalls
      || item.message.toolCalls.length === 0
    ) {
      continue;
    }

    const previousMatchEntryCount = toolCallMatchEntryCount(consumableMatchState);
    const toolCalls = filterToolCallsByConsumableMatchState(
      item.message.toolCalls,
      consumableMatchState,
    );
    if (toolCallMatchEntryCount(consumableMatchState) !== previousMatchEntryCount) {
      hiddenToolCallsByItemId.set(item.id, toolCalls);
    }
  }

  return hiddenToolCallsByItemId;
}

function isToolOnlyRenderItem(item: MessageRenderItem) {
  if (item.message.role !== "assistant" || item.message.knowledgeProposal) return false;

  const hasContent = item.message.content.trim().length > 0;
  const hasThinking = shouldRenderHistoryThinkingBlock(item);
  const hasAttachedKnowledgeProposals = item.attachedKnowledgeProposals.length > 0;
  const hasToolCalls = toolCallsForRenderItem(item).length > 0;

  return !hasContent && !hasThinking && !hasAttachedKnowledgeProposals && hasToolCalls;
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
    if (last && last.role === msg.role) {
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
                  toolCalls: hiddenToolCallsByItemId.has(item.id)
                    ? hiddenToolCallsByItemId.get(item.id)
                    : item.message.toolCalls,
                  attachedKnowledgeProposalCount: item.attachedKnowledgeProposals.length,
                  isKnowledgeProposal: !!item.message.knowledgeProposal,
                })),
            ).map(
              ({
                content: _content,
                thinkingContent: _thinkingContent,
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

const hasThinkingContent = computed(() => props.hasThinking ?? !!props.thinkingText);
const hasVisibleCompletedThinkingContent = computed(() => !shouldHideThinkingBlocks() && hasThinkingContent.value);
const hasVisibleTransientThinkingBlock = computed(() => props.isThinking || hasVisibleCompletedThinkingContent.value);
const hasLiveToolCalls = computed(() => props.activeToolCalls.length > 0);
const hasTransientToolCalls = computed(() => transientToolCalls.value.length > 0);
const hasToolCallHandoff = computed(() => hasTransientToolCalls.value && !hasLiveToolCalls.value);
const hasStreamingContent = computed(() => hasVisibleStreamingText.value || hasLiveToolCalls.value);
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
const hasTransientAssistantMessage = computed(
  () =>
    hasStreamingContent.value
    || hasToolCallHandoff.value
    || hasVisibleTransientThinkingBlock.value
    || isWaitingForResponse.value,
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

  return collectPendingContinuationToolItemIds({
    isStreaming: props.isStreaming,
    lastGroupRole: lastGroup?.role ?? null,
    hasTransientAssistantMessage: hasTransientAssistantMessage.value,
    items:
      lastGroup?.role === "assistant"
        ? lastGroup.items.map((item) => ({
            id: item.id,
            content: item.message.content,
            toolCallCount: toolCallsForRenderItem(item).length,
          }))
        : [],
  });
});

const nonCollapsibleToolItemIds = computed(() => {
  return new Set(pendingContinuationToolItemIds.value);
});

function shouldKeepToolItemExpanded(itemId: string) {
  return nonCollapsibleToolItemIds.value.has(itemId);
}

type HistoryRenderSegment =
  | { type: "thinking"; key: string; content: string; duration?: number }
  | { type: "toolCalls"; key: string; itemId: string; toolCalls: ToolCallDisplay[] }
  | { type: "content"; key: string; content: string }
  | { type: "knowledgeProposal"; key: string; message: ChatMessage };

type TransientRenderSegment =
  | { type: "thinking"; key: string; order: number }
  | { type: "toolCalls"; key: string; order: number; toolCalls: ToolCallDisplay[] }
  | { type: "waiting"; key: string; order: number; withinToolGroup: boolean }
  | { type: "content"; key: string; order: number; content: string };

const TRANSIENT_FALLBACK_ORDER: Record<TransientRenderSegment["type"], number> = {
  thinking: 10,
  toolCalls: 20,
  waiting: 30,
  content: 40,
};

const TRANSIENT_ORDER_TIEBREAK: Record<TransientRenderSegment["type"], number> = {
  thinking: 0,
  toolCalls: 1,
  waiting: 2,
  content: 3,
};

function historyRenderSegments(item: MessageRenderItem): HistoryRenderSegment[] {
  const segments: HistoryRenderSegment[] = [];
  const beforeToolCalls = toolCallsBeforeContentForRenderItem(item);
  const afterToolCalls = toolCallsAfterContentForRenderItem(item);

  if (shouldRenderHistoryThinkingBlock(item)) {
    segments.push({
      type: "thinking",
      key: `${item.id}:thinking`,
      content: item.message.thinkingContent ?? "",
      duration: item.message.thinkingDuration,
    });
  }

  if (beforeToolCalls.length > 0) {
    segments.push({
      type: "toolCalls",
      key: `${item.id}:tools:before`,
      itemId: item.id,
      toolCalls: beforeToolCalls,
    });
  }

  if (item.message.content) {
    segments.push({
      type: "content",
      key: `${item.id}:content`,
      content: item.message.content,
    });
  }

  if (afterToolCalls.length > 0) {
    segments.push({
      type: "toolCalls",
      key: `${item.id}:tools:after`,
      itemId: item.id,
      toolCalls: afterToolCalls,
    });
  }

  for (const proposalMsg of item.attachedKnowledgeProposals) {
    segments.push({
      type: "knowledgeProposal",
      key: proposalMsg.id,
      message: proposalMsg,
    });
  }

  return segments;
}

function firstToolCallOrder(toolCalls: ToolCallDisplay[]) {
  let order = Number.POSITIVE_INFINITY;
  const visit = (items: ToolCallDisplay[]) => {
    for (const toolCall of items) {
      if (typeof toolCall.order === "number" && toolCall.order > 0) {
        order = Math.min(order, toolCall.order);
      }
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };
  visit(toolCalls);
  return Number.isFinite(order) ? order : 0;
}

const transientToolCallsOrder = computed(() => firstToolCallOrder(transientToolCalls.value));

const transientRenderSegments = computed<TransientRenderSegment[]>(() => {
  const segments: TransientRenderSegment[] = [];
  const toolOrder = transientToolCallsOrder.value || TRANSIENT_FALLBACK_ORDER.toolCalls;

  if (hasVisibleTransientThinkingBlock.value) {
    segments.push({
      type: "thinking",
      key: "transient:thinking",
      order: props.thinkingOrder && props.thinkingOrder > 0
        ? props.thinkingOrder
        : TRANSIENT_FALLBACK_ORDER.thinking,
    });
  }

  if (transientToolCalls.value.length > 0) {
    segments.push({
      type: "toolCalls",
      key: "transient:tools",
      order: toolOrder,
      toolCalls: transientToolCalls.value,
    });
  }

  if (isStandaloneWaitingPlaceholder.value) {
    segments.push({
      type: "waiting",
      key: "transient:waiting",
      order: TRANSIENT_FALLBACK_ORDER.waiting,
      withinToolGroup: false,
    });
  }

  if (props.streamingText) {
    segments.push({
      type: "content",
      key: "transient:content",
      order: props.streamingTextOrder && props.streamingTextOrder > 0
        ? props.streamingTextOrder
        : TRANSIENT_FALLBACK_ORDER.content,
      content: props.streamingText,
    });
  }

  return segments.sort((left, right) =>
    left.order - right.order
    || TRANSIENT_ORDER_TIEBREAK[left.type] - TRANSIENT_ORDER_TIEBREAK[right.type],
  );
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

function shouldRightAlignUserMessageGroup(group: Pick<MessageGroup, "role">) {
  return props.variant === "session" && displaySettings.rightAlignUserMessages && group.role === "user";
}

function shouldShowSessionRoundDivider(group: Pick<MessageGroup, "role">, index: number) {
  return props.variant === "session" && group.role === "user" && index > 0;
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
        <div
          v-for="(group, idx) in groupedMessages"
          :key="group.id"
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
            <div
              v-for="item in group.items"
              v-show="shouldRenderItem(item)"
              :key="item.id"
              class="chat-transcript-item-stack"
              :class="[
                `is-${variant}`,
                {
                  'tool-only': isToolOnlyRenderItem(item),
                },
              ]"
              :data-scroll-anchor-id="item.id"
            >
              <template v-if="item.message.role === 'user'">
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

                <div v-if="userMessageDisplayContent(item.message)" class="chat-transcript-plain-text ui-select-text">
                  <template
                    v-for="(segment, segmentIdx) in userMessageDisplaySegments(item.message)"
                    :key="segmentIdx"
                  >
                    <AssetChip v-if="segment.type === 'asset'" :path="segment.value" />
                    <template v-else>{{ segment.value }}</template>
                  </template>
                </div>
              </template>

              <template v-else>
                <KnowledgeProposalCard
                  v-if="item.message.knowledgeProposal"
                  :proposal="item.message.knowledgeProposal"
                  @apply="emit('applyKnowledgeProposal', $event)"
                  @ignore="emit('ignoreKnowledgeProposal', $event)"
                />

                <template v-else>
                  <template
                    v-for="segment in historyRenderSegments(item)"
                    :key="segment.key"
                  >
                    <div v-if="segment.type === 'thinking'" class="chat-transcript-thinking-block">
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

                    <div v-else-if="segment.type === 'toolCalls'" class="chat-transcript-tool-calls-group">
                      <ToolCallCollection
                        :tool-calls="segment.toolCalls"
                        :allow-collapse="!shouldKeepToolItemExpanded(segment.itemId)"
                        :collapse-enabled="!shouldKeepToolItemExpanded(segment.itemId)"
                        @viewport-anchor-start="emitToolViewportAnchorStart"
                        @viewport-anchor-end="emitToolViewportAnchorEnd"
                      >
                        <template #default="{ toolCall }">
                          <ToolCallBlock
                            :tool-call="toolCall"
                            :collapse-enabled="!shouldKeepToolItemExpanded(segment.itemId)"
                            @tool-viewport-anchor-start="emitToolViewportAnchorStart"
                            @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
                          />
                        </template>
                      </ToolCallCollection>
                    </div>

                    <MarkdownRenderer
                      v-else-if="segment.type === 'content'"
                      :content="segment.content"
                      enable-file-refs
                    />

                    <KnowledgeProposalCard
                      v-else-if="segment.type === 'knowledgeProposal'"
                      :proposal="segment.message.knowledgeProposal!"
                      @apply="emit('applyKnowledgeProposal', $event)"
                      @ignore="emit('ignoreKnowledgeProposal', $event)"
                    />
                  </template>
                </template>
              </template>
            </div>
          </div>
        </div>

        <div
          v-if="hasTransientAssistantMessage"
          class="chat-transcript-message assistant transient"
          :class="[
            `is-${variant}`,
            {
              continuation: isStreamingContinuation,
              'waiting-placeholder': isStandaloneWaitingPlaceholder,
            },
          ]"
          data-scroll-anchor-id="__transient__"
        >
          <div
            v-if="!isStreamingContinuation"
            class="chat-transcript-message-role"
            :class="`is-${variant}`"
          >
            {{ assistantLabel }}
          </div>

          <div class="chat-transcript-message-content" :class="`is-${variant}`">
            <template
              v-for="segment in transientRenderSegments"
              :key="segment.key"
            >
              <div v-if="segment.type === 'thinking'" class="chat-transcript-thinking-block">
                <button
                  v-if="variant === 'session'"
                  type="button"
                  class="chat-transcript-thinking-header"
                  :class="{ active: isThinking, 'is-clickable': true }"
                  @click="emit('openThinking', '')"
                >
                  <svg class="chat-transcript-thinking-chevron" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
                    <path d="M6 3l5 5-5 5V3z" />
                  </svg>
                  <template v-if="isThinking">
                    <span class="chat-transcript-thinking-spinner" />
                    <span class="chat-transcript-thinking-title">{{ thinkingActiveLabel }}</span>
                  </template>
                  <template v-else>
                    <span class="chat-transcript-thinking-title">{{ formatThoughtSummary(thinkingDuration) }}</span>
                  </template>
                </button>

                <div v-else class="chat-transcript-thinking-chip" :class="{ active: isThinking }">
                  <span v-if="isThinking" class="chat-transcript-thinking-spinner compact" />
                  <span class="chat-transcript-thinking-title">
                    {{ isThinking ? thinkingActiveLabel : formatThoughtSummary(thinkingDuration) }}
                  </span>
                </div>
              </div>

              <div v-else-if="segment.type === 'toolCalls'" class="chat-transcript-tool-calls-group">
                <ToolCallCollection
                  :tool-calls="segment.toolCalls"
                  :allow-collapse="transientToolCallsAllowCollapse"
                  :collapse-enabled="transientToolCallsCollapseEnabled"
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
                <div v-if="isToolWaitingForResponse" class="chat-transcript-tool-waiting-row">
                  <span class="chat-transcript-thinking-spinner compact" />
                  <span class="chat-transcript-thinking-title">{{ waitingLabel }}</span>
                </div>
              </div>

              <div v-else-if="segment.type === 'waiting'" class="chat-transcript-thinking-block">
                <div class="chat-transcript-thinking-header active">
                  <span class="chat-transcript-thinking-spinner" />
                  <span class="chat-transcript-thinking-title">{{ waitingLabel }}</span>
                </div>
              </div>

              <MarkdownRenderer
                v-else-if="segment.type === 'content'"
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
