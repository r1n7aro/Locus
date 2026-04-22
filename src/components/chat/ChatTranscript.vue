<script setup lang="ts">
import { computed, ref } from "vue";
import type { ChatMessage, ToolCallDisplay, ToolCallInfo, UserIntentMeta } from "../../types";
import {
  collectPendingContinuationToolItemIds,
  shouldShowAssistantContinuation,
  shouldShowWaitingPlaceholder,
} from "../../composables/chatViewStability";
import {
  buildMessageToolCalls,
  collectToolCallDisplayIds,
  filterToolCallsByActiveIds,
  mergeSequentialAssistantToolCalls,
} from "../../composables/toolCallBatches";
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
}

const props = withDefaults(defineProps<{
  messages: ChatMessage[];
  streamingText: string;
  isStreaming: boolean;
  isThinking: boolean;
  hasThinking?: boolean;
  thinkingText?: string;
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
  pinnedToolMessageId?: string | null;
}>(), {
  variant: "embedded",
  hasThinking: undefined,
  thinkingText: "",
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
  pinnedToolMessageId: null,
});

const emit = defineEmits<{
  (e: "applyKnowledgeProposal", proposalId: string): void;
  (e: "ignoreKnowledgeProposal", proposalId: string): void;
  (e: "openThinking", content: string): void;
  (e: "openImage", src: string): void;
  (e: "scroll", event: Event): void;
  (e: "contentClick", event: MouseEvent): void;
  (e: "contentMouseover", event: MouseEvent): void;
  (e: "contentMouseout", event: MouseEvent): void;
}>();

const scrollRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);

defineExpose({
  getScrollElement: () => scrollRef.value,
  getContentElement: () => contentRef.value,
  scrollToBottom() {
    const element = scrollRef.value;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
  },
});

const ASSET_REF_RE = /@((?:[^\s@]+\/)+[^\s@]+)/g;

interface ContentSegment {
  type: "text" | "asset";
  value: string;
}

function parseAssetRefs(text: string): ContentSegment[] {
  const segments: ContentSegment[] = [];
  let lastIndex = 0;
  ASSET_REF_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = ASSET_REF_RE.exec(text)) !== null) {
    if (match.index > lastIndex) {
      segments.push({ type: "text", value: text.slice(lastIndex, match.index) });
    }
    segments.push({ type: "asset", value: match[1] });
    lastIndex = ASSET_REF_RE.lastIndex;
  }
  if (lastIndex < text.length) {
    segments.push({ type: "text", value: text.slice(lastIndex) });
  }
  return segments;
}

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
      label: `Skill: ${skill.name}`,
      kind: "skill",
    });
  }

  return badges;
}

function messageIntentBadges(message: ChatMessage) {
  return buildIntentBadges(message.intentMeta ?? null);
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

const activeToolCallIds = computed(() => collectToolCallDisplayIds(props.activeToolCalls));

function messageToolCallsForDisplay(message: Pick<ChatMessage, "toolCalls">) {
  return filterToolCallsByActiveIds(message.toolCalls, activeToolCallIds.value);
}

const groupedMessages = computed<MessageGroup[]>(() => {
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

  for (const item of flatItems) {
    if (!item.message.knowledgeProposal) continue;
    const nextRequestTool = flatItems.find((candidate) =>
      candidate.order > item.order
      && !candidate.message.knowledgeProposal
      && hasKnowledgeMutationToolCall(candidate.message),
    );
    const prevRequestTool = [...flatItems].reverse().find((candidate) =>
      candidate.order < item.order
      && !candidate.message.knowledgeProposal
      && hasKnowledgeMutationToolCall(candidate.message),
    );
    const target = nextRequestTool ?? prevRequestTool;
    if (!target) continue;
    target.attachedKnowledgeProposals.push(item.message);
    item.hidden = true;
  }

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
                  toolCalls: messageToolCallsForDisplay(item.message),
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
});

const hasThinkingContent = computed(() => props.hasThinking ?? !!props.thinkingText);
const hasStreamingContent = computed(() => !!props.streamingText || props.activeToolCalls.length > 0);
const isWaitingForResponse = computed(
  () => shouldShowWaitingPlaceholder({
    isStreaming: props.isStreaming,
    hasStreamingContent: hasStreamingContent.value,
    isThinking: props.isThinking,
    hasThinkingContent: hasThinkingContent.value,
  }),
);
const hasTransientAssistantMessage = computed(
  () => hasStreamingContent.value || props.isThinking || hasThinkingContent.value || isWaitingForResponse.value,
);

const isStreamingContinuation = computed(() => {
  const groups = groupedMessages.value;
  return shouldShowAssistantContinuation(
    groups.length > 0 ? groups[groups.length - 1].role : null,
    hasTransientAssistantMessage.value,
  );
});

const pendingContinuationToolItemIds = computed(() => {
  const groups = groupedMessages.value;
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
  const ids = new Set(pendingContinuationToolItemIds.value);
  if (props.pinnedToolMessageId) {
    ids.add(props.pinnedToolMessageId);
  }
  return ids;
});

const collapseCompletedToolCalls = computed(() => !props.isStreaming);

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

function toolCallsForRenderItem(item: Pick<MessageRenderItem, "message" | "displayToolCalls">) {
  return buildMessageToolCalls(
    { toolCalls: item.displayToolCalls ?? messageToolCallsForDisplay(item.message) },
    toolOutputMap.value,
  );
}

function shouldRenderItem(item: MessageRenderItem) {
  if (item.message.knowledgeProposal) return true;

  if (item.message.role === "user") {
    return !!(
      item.message.content
      || (props.showUserImages && item.message.images && item.message.images.length > 0)
      || (props.enableIntentBadges && messageIntentBadges(item.message).length > 0)
    );
  }

  return !!(
    item.message.content
    || item.message.thinkingContent
    || toolCallsForRenderItem(item).length > 0
    || item.attachedKnowledgeProposals.length > 0
  );
}

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

function emitContentMouseover(event: MouseEvent) {
  emit("contentMouseover", event);
}

function emitContentMouseout(event: MouseEvent) {
  emit("contentMouseout", event);
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
    @mouseover="emitContentMouseover"
    @mouseout="emitContentMouseout"
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
              'before-continuation': isStreamingContinuation && idx === groupedMessages.length - 1,
              'compact-handoff': group.items.some((item) => isCompactHandoffMessage(item.message)),
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
              :class="`is-${variant}`"
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

                <div v-if="item.message.content" class="chat-transcript-plain-text ui-select-text">
                  <template
                    v-for="(segment, segmentIdx) in userContentMode === 'asset'
                      ? parseAssetRefs(item.message.content)
                      : [{ type: 'text', value: item.message.content }]"
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
                  <div v-if="item.message.thinkingContent" class="chat-transcript-thinking-block">
                    <button
                      v-if="variant === 'session'"
                      type="button"
                      class="chat-transcript-thinking-header is-clickable"
                      @click="emit('openThinking', item.message.thinkingContent)"
                    >
                      <svg class="chat-transcript-thinking-chevron" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
                        <path d="M6 3l5 5-5 5V3z" />
                      </svg>
                      <span class="chat-transcript-thinking-title">
                        {{ formatThoughtSummary(item.message.thinkingDuration) }}
                      </span>
                    </button>

                    <div v-else class="chat-transcript-thinking-chip">
                      <span class="chat-transcript-thinking-title">
                        {{ formatThoughtSummary(item.message.thinkingDuration) }}
                      </span>
                    </div>
                  </div>

                  <div v-if="toolCallsForRenderItem(item).length > 0" class="chat-transcript-tool-calls-group">
                    <ToolCallCollection
                      :tool-calls="toolCallsForRenderItem(item)"
                      :allow-collapse="!nonCollapsibleToolItemIds.has(item.id)"
                      :collapse-enabled="collapseCompletedToolCalls"
                    >
                      <template #default="{ toolCall }">
                        <ToolCallBlock :tool-call="toolCall" :collapse-enabled="collapseCompletedToolCalls" />
                      </template>
                    </ToolCallCollection>
                  </div>

                  <MarkdownRenderer
                    v-if="item.message.content"
                    :content="item.message.content"
                    enable-file-refs
                  />

                  <KnowledgeProposalCard
                    v-for="proposalMsg in item.attachedKnowledgeProposals"
                    :key="proposalMsg.id"
                    :proposal="proposalMsg.knowledgeProposal!"
                    @apply="emit('applyKnowledgeProposal', $event)"
                    @ignore="emit('ignoreKnowledgeProposal', $event)"
                  />
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
              'waiting-placeholder': isWaitingForResponse,
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
            <div v-if="isWaitingForResponse" class="chat-transcript-thinking-block">
              <div class="chat-transcript-thinking-header active">
                <span class="chat-transcript-thinking-spinner" />
                <span class="chat-transcript-thinking-title">{{ waitingLabel }}</span>
              </div>
            </div>

            <div v-if="isThinking || hasThinkingContent" class="chat-transcript-thinking-block">
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

            <div v-if="activeToolCalls.length > 0" class="chat-transcript-tool-calls-group">
              <ToolCallCollection :tool-calls="activeToolCalls" :allow-collapse="false" :collapse-enabled="false">
                <template #default="{ toolCall }">
                  <ToolCallBlock :tool-call="toolCall" :collapse-enabled="false" />
                </template>
              </ToolCallCollection>
            </div>

            <MarkdownRenderer
              v-if="streamingText"
              :content="streamingText"
              cursor
              enable-file-refs
            />
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.chat-transcript-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
}

.chat-transcript-content {
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
  border-top: 1px solid var(--msg-divider);
}

.chat-transcript-message.is-session.assistant.transient.continuation {
  border-top: none;
}

.chat-transcript-message.is-session.user {
  background: var(--msg-user-bg);
  border-top: 1px solid var(--msg-divider);
  border-bottom: 1px solid var(--msg-divider);
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
}
</style>
