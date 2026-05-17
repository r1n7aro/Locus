# 工具折叠与会话消息渲染逻辑导出

导出时间：2026-05-16

仓库：`C:\Projects\locus`

目的：把当前工具折叠、会话消息渲染、流式事件归约、持久化与导出相关逻辑整理成一个 Web 端可独立阅读的文档。Web 端 GPT Pro 无法访问本地文件，因此本文末尾直接嵌入关键源码与测试片段。

## 阅读结论

当前实现把同一批工具调用同时放在三条链路里处理：后端持久化消息、前端 live stream 状态、ChatTranscript 的 handoff 过渡状态。工具折叠问题多数来自“同一个工具调用当前由谁负责展示”的判定在多个层级里完成，且判定依赖 id、语义 fingerprint、render order、动画时机与滚动稳定状态。

架构上最值得优先收敛的是：先生成单一的 conversation render timeline，再在 timeline 之后做折叠策略。这样 tool call 的归属、排序、去重、handoff、折叠展开状态可以从 UI 组件内部迁出，测试也能覆盖纯函数级别的边界。

## 核心入口

- `src/components/chat/ChatTranscript.vue`：消息渲染的中心入口，合并历史消息、实时 render parts、active tool calls、handoff 工具调用、等待状态与滚动稳定逻辑。
- `src/components/ToolCallCollection.vue`：一批工具调用的折叠入口，负责批量 header、折叠动画、collapse finished 事件。
- `src/components/ToolCallBlock.vue`：单个工具调用块，负责单工具展开状态、嵌套工具、subagent 运行时自动展开与结束后自动折叠。
- `src/composables/toolCallBatches.ts`：历史消息中的 tool call 构建、工具批次摘要、跨消息合并、去重与 fingerprint 匹配。
- `src/composables/useStreamReducer.ts`：后端 stream event 到前端消息状态的 reducer，生成 live render parts、active tool calls、hidden tool result messages。
- `src/stores/chat.ts`：应用 reducer mutation，维护 session messages、activeToolCalls、liveRenderParts、run 状态和 session 切换清理。
- `src-tauri/src/agent/instance/mod.rs`：LLM 响应解析与 stream event 发射，包含 render order tracker、tool call round done 和 final assistant done。
- `src-tauri/src/session/store.rs`：消息表、metadata_json、render_parts 持久化、历史 migration、消息查询。
- `src-tauri/src/session/gateway.rs`：stream event 合并、前端事件发送、事件持久化入队。
- `src-tauri/src/commands/session.rs`：会话上下文保存与 Markdown/JSON 导出。
- `src/components/MarkdownRenderer.vue` 与 `src/composables/markdownRender.ts`：普通消息正文、工具输出 Markdown、代码块、表格、链接和文内引用的最终渲染层。

## 当前数据流

````
LLM / agent instance
  -> StreamEvent.TextDelta / ThinkingDelta / ToolCallStart / ToolCallDone / ToolCallRoundDone / Done
  -> session gateway emits stream-event and persists event payload
  -> frontend chat store calls useStreamReducer.reduceStreamEvent
  -> store state updates messages, liveRenderParts, activeToolCalls, streaming text/thinking
  -> ChatView passes state into ChatTranscript
  -> ChatTranscript builds historyRenderSegments + transientRenderSegments
  -> ToolCallCollection renders each visible batch and decides collapsibility
  -> ToolCallBlock renders each tool, nested tool calls, output, progress, error
````

## 关键数据模型

- `ToolCallInfo`：历史/持久化工具调用模型。字段包括 `id`、`name`、`arguments`、`status`、`order`、`outcome`、`recordedOutput`、`serverToolOutput`、`nestedToolCalls`。
- `ToolCallDisplay`：前端展示态工具调用模型。字段包括 `id`、`name`、`arguments`、`status`、`output`、`progress`、`startedAt`、`completedAt`、`nestedToolCalls`。
- `AssistantRenderPart`：助手消息内部的规范渲染序列。类型包括 `thinking`、`text`、`toolCall`、`knowledgeProposal`，每个 part 带 `order`，并可带 `runId` 与 `seq`。
- `ChatMessage`：会话消息模型。除了 `role/content`，还可能带 `toolCalls`、`toolCallId`、`thinkingContent`、`renderParts`、`assetRefs`、`images`、`includeInPrompt`。
- `toolOutputMap`：`ChatTranscript` 从 `role: "tool"` 消息建立 `toolCallId -> content` 映射，用来补齐历史 tool call 的输出。
- `activeToolCalls`：前端实时运行态工具调用列表，由 stream reducer 根据 start/done/progress/delta 事件维护。
- `liveRenderParts`：前端实时助手渲染序列，由 stream reducer 维护，直到后端 Done 和历史消息落地。

## 消息渲染逻辑

`ChatTranscript` 先过滤消息，再按 assistant 与紧随其后的 tool result 消息分组。历史 assistant 消息优先使用 `message.renderParts` 生成段落；旧数据缺少 renderParts 时使用 `thinkingOrder`、`contentOrder`、`toolCall.order` 等 legacy 顺序兜底。

历史渲染段分为 `thinking`、`text`、`toolCalls`、`knowledgeProposal`。工具段由 `buildMessageToolCalls` 从 assistant 消息和随后的 tool result 消息中构建，并通过 `mergeSequentialAssistantToolGroups` 合并连续的纯工具轮次，避免每个 assistant tool-only round 独立显示成多个批次。

实时渲染段由 `liveRenderParts` 与 `activeToolCalls` 生成。`ChatTranscript` 在 active 工具结束但最终历史消息尚未稳定时创建 `toolCallHandoff`，临时保留工具块，防止从 live 状态切到 history 状态时闪烁、重复或滚动跳动。

`MarkdownRenderer` 是普通 assistant/user 正文和工具输出的最终展示层。`ChatTranscript` 只决定消息段、顺序和容器，正文内容进入 `MarkdownRenderer` 后再完成 Markdown 规范化、链接处理、代码块、表格、文件引用等渲染。

## 工具折叠规则

批量折叠由 `summarizeToolCallBatch` 与 `ToolCallCollection` 共同决定。`summarizeToolCallBatch(toolCalls, compactEnabled)` 的核心规则是：启用 compact、工具数至少 2、没有 running 工具时，批次可以折叠。失败、取消、中断等终态工具仍可参与折叠。

````
const canCollapse = compactEnabled && total >= 2 && runningCount === 0
````

`ToolCallCollection` 还接收 `allowCollapse` 和 `collapseEnabled`。只要不可折叠，collection 会展开。可折叠时，它用本地 `collapsed` 状态控制内容高度动画，并在 transition end 后触发 `collapse-finished`。

`ToolCallBlock` 有自己的单工具展开状态。subagent 工具运行中默认展开，结束后自动折叠；手动展开后会设置 `userExpanded`，避免自动逻辑覆盖用户操作。嵌套工具列表会继续向下传递 `collapseEnabled`。

## Handoff 与滚动稳定

handoff 的目标是在 live 工具列表消失和 history 消息出现之间保持视觉连续。`ChatTranscript` 在 `activeToolCalls` 从非空变为空时复制一个工具快照，并等待历史消息或最终文本到达。期间会把部分历史工具调用标记为 promotable，使其继续走 transient 视觉路径。

handoff 结束时机由多种条件共同控制：历史匹配、流结束、可见文本出现、最少保留时间、两次 requestAnimationFrame、折叠动画 transitionend、滚动 quiet mode。这个逻辑能减少闪烁，也扩大了 race condition 面积。

`chatViewStability.ts` 负责等待占位、continuation pin、running nested tool 判断、streaming scroll scheduling 等辅助判断。它和 `ChatView` 中的 scroll anchor / resize observer 一起保护聊天窗口在流式更新时稳定。

## 会话导出逻辑

会话导出在后端 `src-tauri/src/commands/session.rs`。导出时会读取 session messages，输出 JSON 与 Markdown。对于缺失字段，现有逻辑会显式写出 `empty`，这与仓库 AGENTS.md 对旧 schema 导出的要求一致。tool calls、renderParts、thinking、images、assetRefs、includeInPrompt 等字段都会参与导出或 fallback formatting。

## 实现合理性判断

- 优点：后端已经保留 render order 与 renderParts，能够表达 thinking、text、tool call、knowledge proposal 的真实交错顺序。
- 优点：前端 reducer 让 live 状态和历史消息状态都落在统一 mutation 里，测试入口较清晰。
- 优点：`toolCallBatches.ts` 已经把历史工具调用构建、批次摘要、fingerprint 去重抽出来，具备继续收敛的基础。
- 风险：同一工具调用的可见性由 history、live、handoff 三层共同控制，重复隐藏和保留逻辑分散在 `ChatTranscript` 内部。
- 风险：工具去重同时依赖 id 和 semantic fingerprint。fingerprint 可以兜底 id 漂移，也可能在连续相同工具调用时误隐藏。
- 风险：render order 的来源包括后端 seq、持久化 renderParts、legacy order、前端 fallback。旧数据迁移或 partial stream 情况下容易出现顺序漂移。
- 风险：折叠状态既来自 display setting，也来自 collection 本地状态、block 本地状态、handoff 自动折叠、用户手动展开。状态边界偏散。
- 风险：handoff 使用 nextTick、requestAnimationFrame、timer、transitionend、timeout 与滚动 quiet mode 协同，行为正确性受渲染时机影响。
- 风险：工具输出可来自 `role:"tool"` 消息、`recordedOutput`、`serverToolOutput`，需要固定优先级并用测试锁住。

## 建议重构方向

- 新增纯函数层 `ConversationRenderTimeline` 或 `useToolRenderTimeline`。输入是历史 messages、liveRenderParts、activeToolCalls、handoff lease、display settings；输出是唯一的 render segment list。
- 把“谁拥有这个工具调用的展示权”从 UI 组件迁到 timeline 层。每个 tool segment 只能有一个 owner：`history`、`live` 或 `handoff`。
- 把折叠策略放在 timeline 之后。折叠策略只消费最终工具批次、display setting、用户展开状态，避免参与去重和 handoff。
- 引入稳定 `toolInstanceKey`。优先使用 `runId + toolCallId` 或后端持久化 display id；fingerprint 只作为 legacy migration 或修复旧数据的 fallback。
- 把用户展开状态存为 `Map<segmentKey, expanded/collapsed>`。自动折叠只能在 segment identity 初次出现或状态终结时写入，不能覆盖用户手动选择。
- 将 handoff 简化成 ownership lease。live 工具结束后由 handoff lease 继续拥有展示，直到对应 history part 出现或 run terminal；timeline 层决定展示同一段工具内容的哪个 owner。
- 把 `ChatTranscript` 中 handoff、visible match、retained match、promotable history 等逻辑拆成可测试 composable，组件只负责渲染和事件转发。
- 为复杂边界建立 fixture 测试：工具结束先于最终文本、没有最终文本、server tool only、嵌套 subagent、连续相同调用、id 漂移、取消/中断、session 切换、compact on/off、hideThinkingBlocks on/off。

## 调试入口

`src/services/toolCollapseTrace.ts` 提供本地调试开关。浏览器控制台执行以下代码后刷新或继续对话，可以看到相关 trace。

````js
localStorage.setItem('locus.toolCollapseTrace', 'handoff')
localStorage.setItem('locus.toolCollapseTrace', 'all')
localStorage.setItem('locus.toolCollapseTrace', 'waiting')
localStorage.removeItem('locus.toolCollapseTrace')
````

## 测试覆盖现状

- `src/__tests__/toolCallBatches.test.ts`：覆盖历史工具调用构建、order、nested、id/fingerprint 去重、collapse 摘要、连续 tool-only assistant 合并。
- `src/__tests__/useStreamReducer.test.ts`：覆盖 stream event reducer、renderParts、工具结果隐藏消息、取消/重置等。
- `src/__tests__/chatViewStability.test.ts`：覆盖 continuation pin、等待占位、nested running tool、scroll scheduler。
- `src/__tests__/chatSidebarLayout.test.ts`：覆盖 ChatTranscript/ChatView 的等待、handoff、工具折叠展示行为。
- `src/__tests__/displaySettingsLayout.test.ts`：覆盖显示设置里的 compact tool calls 等开关。

## Web 端修复建议任务

- 第一步：从附录源码中抽出 timeline 纯函数，先复刻现有行为并用现有测试对齐。
- 第二步：增加 owner 断言，确保同一 toolInstanceKey 在最终 render segments 中只出现一次。
- 第三步：把 collapse eligibility 的单元测试从 `ToolCallCollection` UI 里下沉到纯函数。
- 第四步：把 handoff 的 release 条件改为 timeline owner 变化，减少基于动画事件的业务判断。
- 第五步：保留现有 visual transition，但让 transition 只影响显示动画和 scroll quiet mode。

## 源码附录

以下源码直接来自当前工作区，供 Web 端完整阅读。完整文件用于核心前端逻辑；后端与大组件使用与工具折叠、消息渲染、持久化、导出直接相关的片段。

### src/components/chat/ChatTranscript.vue

行数：2611

````vue
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
  (e: "contentContextmenu", event: MouseEvent): void;
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
  (activeToolCallCount, previousActiveToolCallCount) => {
    if (activeToolCallCount > 0 && toolCallHandoff.value) {
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

const shouldHidePromotedHistoryToolCalls = computed(() =>
  promotableHistoryToolCalls.value.toolCalls.length > 0
  && (props.activeToolCalls.length > 0 || !!toolCallHandoff.value?.collapseArmed),
);

watch(shouldHidePromotedHistoryToolCalls, (next, previous) => {
  traceToolCollapse("promotedHistoryToolCallsVisibilityChanged", {
    previous,
    next,
    promotedToolCallCount: promotableHistoryToolCalls.value.toolCalls.length,
    promotedToolCallIds: promotableHistoryToolCalls.value.toolCalls.map((toolCall) => toolCall.id),
    activeToolCallCount: props.activeToolCalls.length,
    activeToolCallIds: props.activeToolCalls.map((toolCall) => toolCall.id),
    hasHandoff: !!toolCallHandoff.value,
    handoffCollapseArmed: toolCallHandoff.value?.collapseArmed ?? false,
    isStreaming: props.isStreaming,
  });
});

const historyHiddenToolCallMatchState = computed<ToolCallMatchState>(() => {
  if (!shouldHidePromotedHistoryToolCalls.value) {
    return activeToolCallMatchState.value;
  }
  return mergeToolCallMatchStates(
    activeToolCallMatchState.value,
    promotableHistoryToolCalls.value.toolCallMatchState,
  );
});

const groupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(historyHiddenToolCallMatchState.value));

const shouldRenderPromotedHistoryToolCallsInTransient = computed(() =>
  promotableHistoryToolCalls.value.toolCalls.length > 0
  && (props.activeToolCalls.length > 0 || !!toolCallHandoff.value?.collapseArmed),
);

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

type HistoryRenderSegment =
  | { type: "thinking"; key: string; part: AssistantRenderPart; content: string; duration?: number }
  | { type: "toolCalls"; key: string; part: Extract<AssistantRenderPart, { kind: "toolCall" }>; itemId: string; itemIds: string[]; toolCalls: ToolCallDisplay[] }
  | { type: "content"; key: string; part: AssistantRenderPart; content: string }
  | { type: "knowledgeProposal"; key: string; part: AssistantRenderPart; message: ChatMessage };

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

  let hasRenderedToolSegment = segments.some((segment) => segment.type === "toolCalls");
  const promotedToolCalls = promotableHistoryToolCalls.value.toolCalls;
  if (shouldRenderPromotedHistoryToolCallsInTransient.value && promotedToolCalls.length > 0) {
    const promotedPart = transientToolHandoffPart(transientToolCalls.value);
    if (promotedPart) {
      const firstToolSegmentIndex = segments.findIndex((segment) => segment.type === "toolCalls");
      const firstContentSegmentIndex = segments.findIndex((segment) => segment.type === "content");
      const candidateToolSegment = firstToolSegmentIndex >= 0 ? segments[firstToolSegmentIndex] : null;
      const firstToolSegment = candidateToolSegment?.type === "toolCalls" ? candidateToolSegment : null;

      if (firstToolSegment && (firstContentSegmentIndex < 0 || firstToolSegmentIndex < firstContentSegmentIndex)) {
        const mergedToolCalls = mergeToolCallDisplaysWithoutDuplicates(
          promotedToolCalls,
          firstToolSegment.toolCalls,
        );
        firstToolSegment.key = transientToolSegmentKey(mergedToolCalls);
        firstToolSegment.part = promotedPart;
        firstToolSegment.toolCalls = mergedToolCalls;
        firstToolSegment.allowCollapse = true;
        firstToolSegment.collapseEnabled = true;
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
          collapseEnabled: true,
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
                  :allow-collapse="segment.allowCollapse"
                  :collapse-enabled="segment.collapseEnabled"
                  :animate-collapse-on-mount="segment.animateCollapseOnMount"
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

````

### src/components/ToolCallCollection.vue

行数：466

````vue
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { summarizeToolCallBatch } from "../composables/toolCallBatches";
import { t } from "../i18n";
import { logToolCollapseTrace } from "../services/toolCollapseTrace";

import type { ToolCallDisplay } from "../types";

const props = withDefaults(defineProps<{
  toolCalls: ToolCallDisplay[];
  allowCollapse?: boolean;
  collapseEnabled?: boolean;
  animateCollapseOnMount?: boolean;
}>(), {
  allowCollapse: true,
  collapseEnabled: true,
  animateCollapseOnMount: false,
});
const emit = defineEmits<{
  (e: "collapseFinished"): void;
  (e: "viewportAnchorStart", anchor: HTMLElement): void;
  (e: "viewportAnchorEnd", anchor: HTMLElement): void;
}>();

const { state: displaySettings } = useDisplaySettings();
const startsExpandedForCollapseAnimation =
  props.animateCollapseOnMount
  && summarizeToolCallBatch(
    props.toolCalls,
    displaySettings.compactToolCalls && props.allowCollapse && props.collapseEnabled,
  ).canCollapse;
const expanded = ref(startsExpandedForCollapseAnimation);
const panelVisible = ref(false);
const panelLeaving = ref(false);
const summaryRef = ref<HTMLElement | null>(null);
const panelTransitionCleanup = new WeakMap<HTMLElement, () => void>();

const PANEL_TRANSITION = [
  "height 320ms cubic-bezier(0.2, 0, 0, 1)",
  "opacity 220ms ease",
  "transform 320ms cubic-bezier(0.2, 0, 0, 1)",
].join(", ");
const PANEL_TRANSITION_TIMEOUT_MS = 380;

const batchState = computed(() =>
  summarizeToolCallBatch(
    props.toolCalls,
    displaySettings.compactToolCalls && props.allowCollapse && props.collapseEnabled,
  ),
);

const batchSummary = computed(() => {
  const { total, errorCount, interruptedCount } = batchState.value;
  if (errorCount > 0 && interruptedCount > 0) {
    return t("tool.batch.summaryWithIssues", total, errorCount, interruptedCount);
  }
  if (errorCount > 0) {
    return t("tool.batch.summaryWithErrors", total, errorCount);
  }
  if (interruptedCount > 0) {
    return t("tool.batch.summaryWithInterrupted", total, interruptedCount);
  }
  return t("tool.batch.summary", total);
});

const toggleLabel = computed(() =>
  expanded.value ? t("tool.batch.collapse") : t("tool.batch.expand"),
);

const summaryOpen = computed(() =>
  batchState.value.canCollapse && (expanded.value || panelVisible.value),
);

function traceCollection(event: string, detail?: Record<string, unknown>) {
  logToolCollapseTrace("tool-collection", event, {
    firstToolCallId: props.toolCalls[0]?.id ?? "",
    total: props.toolCalls.length,
    allowCollapse: props.allowCollapse,
    collapseEnabled: props.collapseEnabled,
    canCollapse: batchState.value.canCollapse,
    expanded: expanded.value,
    panelVisible: panelVisible.value,
    panelLeaving: panelLeaving.value,
    ...detail,
  });
}

function clearPanelTransitionListener(element: HTMLElement) {
  const cleanup = panelTransitionCleanup.get(element);
  cleanup?.();
  panelTransitionCleanup.delete(element);
}

function preparePanelTransition(element: HTMLElement) {
  clearPanelTransitionListener(element);
  element.style.overflow = "hidden";
  element.style.transformOrigin = "top center";
  element.style.willChange = "height, opacity, transform";
  element.style.transition = PANEL_TRANSITION;
}

function resetPanelTransition(element: HTMLElement) {
  clearPanelTransitionListener(element);
  element.style.height = "";
  element.style.opacity = "";
  element.style.transform = "";
  element.style.overflow = "";
  element.style.transformOrigin = "";
  element.style.willChange = "";
  element.style.transition = "";
}

function queuePanelTransition(element: HTMLElement, done: () => void) {
  let finished = false;

  const complete = () => {
    if (finished) return;
    finished = true;
    clearPanelTransitionListener(element);
    done();
  };

  const onTransitionEnd = (event: Event) => {
    const transitionEvent = event as TransitionEvent;
    if (transitionEvent.target !== element || transitionEvent.propertyName !== "height") return;
    complete();
  };

  const timeoutId = setTimeout(complete, PANEL_TRANSITION_TIMEOUT_MS);
  element.addEventListener("transitionend", onTransitionEnd);
  panelTransitionCleanup.set(element, () => {
    clearTimeout(timeoutId);
    element.removeEventListener("transitionend", onTransitionEnd);
  });
}

function runOnNextFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

function emitViewportAnchorEnd() {
  const anchor = summaryRef.value;
  if (anchor) emit("viewportAnchorEnd", anchor);
}

function toggleExpanded() {
  const anchor = summaryRef.value;
  if (anchor) emit("viewportAnchorStart", anchor);
  expanded.value = !expanded.value;
}

onMounted(() => {
  if (!startsExpandedForCollapseAnimation) return;
  traceCollection("animateCollapseOnMount");
  runOnNextFrame(() => {
    expanded.value = false;
  });
});

function onPanelEnter(element: Element, done: () => void) {
  const panel = element as HTMLElement;
  traceCollection("panelEnter", {
    heightBefore: panel.scrollHeight,
  });
  panelLeaving.value = false;
  panelVisible.value = true;
  preparePanelTransition(panel);
  panel.style.height = "0px";
  panel.style.opacity = "0";
  panel.style.transform = "translateY(-4px) scaleY(0.97)";
  void panel.offsetHeight;
  queuePanelTransition(panel, done);
  runOnNextFrame(() => {
    panel.style.height = `${panel.scrollHeight}px`;
    panel.style.opacity = "1";
    panel.style.transform = "translateY(0) scaleY(1)";
  });
}

function onPanelAfterEnter(element: Element) {
  traceCollection("panelAfterEnter", {
    heightAfter: (element as HTMLElement).scrollHeight,
  });
  panelLeaving.value = false;
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
}

function onPanelEnterCancelled(element: Element) {
  traceCollection("panelEnterCancelled");
  panelLeaving.value = false;
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
}

function onPanelLeave(element: Element, done: () => void) {
  const panel = element as HTMLElement;
  traceCollection("panelLeave", {
    heightBefore: panel.scrollHeight,
  });
  panelLeaving.value = true;
  panelVisible.value = true;
  preparePanelTransition(panel);
  panel.style.height = `${panel.scrollHeight}px`;
  panel.style.opacity = "1";
  panel.style.transform = "translateY(0) scaleY(1)";
  void panel.offsetHeight;
  queuePanelTransition(panel, done);
  runOnNextFrame(() => {
    panel.style.height = "0px";
    panel.style.opacity = "0";
    panel.style.transform = "translateY(-4px) scaleY(0.97)";
  });
}

function onPanelAfterLeave(element: Element) {
  traceCollection("panelAfterLeave");
  panelVisible.value = false;
  panelLeaving.value = false;
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
  emit("collapseFinished");
}

function onPanelLeaveCancelled(element: Element) {
  traceCollection("panelLeaveCancelled");
  panelLeaving.value = false;
  panelVisible.value = true;
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
}

watch(
  () => ({
    firstId: props.toolCalls[0]?.id ?? "",
    total: props.toolCalls.length,
    allowCollapse: props.allowCollapse,
    collapseEnabled: props.collapseEnabled,
    canCollapse: batchState.value.canCollapse,
    summaryOpen: summaryOpen.value,
  }),
  (next, prev) => {
    traceCollection("stateChanged", {
      previous: prev ?? null,
      next,
    });
  },
  { immediate: true },
);

watch(expanded, (value, previousValue) => {
  traceCollection("expandedChanged", {
    previous: previousValue,
    next: value,
  });
});

watch(
  () => ({
    firstId: props.toolCalls[0]?.id ?? "",
    canCollapse: batchState.value.canCollapse,
  }),
  (next, prev) => {
    traceCollection("collapseResetCheck", {
      previous: prev ?? null,
      next,
    });
    if (!prev) return;
    if (next.firstId !== prev.firstId) {
      expanded.value = false;
    }
  },
  { immediate: true },
);
</script>

<template>
  <div
    class="tool-call-collection"
    :class="{
      'is-collapsible': batchState.canCollapse,
      'is-expanded': batchState.canCollapse && summaryOpen,
      'is-collapsing': batchState.canCollapse && panelLeaving,
    }"
  >
    <button
      v-if="batchState.canCollapse"
      ref="summaryRef"
      type="button"
      class="tool-call-batch-summary ui-select-none"
      :class="{ open: summaryOpen, closing: panelLeaving }"
      :title="toggleLabel"
      :aria-label="toggleLabel"
      :aria-expanded="expanded"
      @click="toggleExpanded"
    >
      <span class="tool-call-batch-chevron" :class="{ open: expanded }" aria-hidden="true">
        <svg viewBox="0 0 12 12" width="10" height="10">
          <path
            d="M4 2.5L8 6 4 9.5"
            fill="none"
            stroke="currentColor"
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="1.5"
          />
        </svg>
      </span>
      <span class="tool-call-batch-title">{{ batchSummary }}</span>
    </button>

    <Transition
      :css="false"
      @enter="onPanelEnter"
      @after-enter="onPanelAfterEnter"
      @enter-cancelled="onPanelEnterCancelled"
      @leave="onPanelLeave"
      @after-leave="onPanelAfterLeave"
      @leave-cancelled="onPanelLeaveCancelled"
    >
      <div
        v-if="!batchState.canCollapse || expanded"
        class="tool-call-collection-panel"
      >
        <div
          class="tool-call-collection-list"
          :class="{
            'with-summary': batchState.canCollapse,
            open: batchState.canCollapse && summaryOpen,
            closing: batchState.canCollapse && panelLeaving,
          }"
        >
          <template v-for="toolCall in toolCalls" :key="toolCall.id">
            <slot :tool-call="toolCall" />
          </template>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.tool-call-collection {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tool-call-collection.is-expanded {
  gap: 0;
}

.tool-call-collection-panel {
  min-width: 0;
}

.tool-call-batch-summary {
  appearance: none;
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 30px;
  padding: 5px 10px 5px 23px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background 0.18s, border-color 0.18s, border-radius 0.24s, color 0.15s;
}

.tool-call-batch-summary:hover,
.tool-call-batch-summary:focus-visible {
  background: color-mix(in srgb, var(--hover-bg) 72%, var(--msg-assistant-bg));
  border-color: color-mix(in srgb, var(--accent-color) 18%, var(--border-color));
}

.tool-call-batch-summary.open {
  border-color: color-mix(in srgb, var(--accent-color) 26%, var(--border-color));
  border-bottom-color: color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px 8px 0 0;
  background: color-mix(in srgb, var(--msg-assistant-bg) 76%, var(--panel-bg) 24%);
}

.tool-call-batch-summary.open.closing {
  border-color: transparent;
  border-radius: 6px;
  background: transparent;
}

.tool-call-batch-summary:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent-color) 44%, transparent);
  outline-offset: 1px;
}

.tool-call-batch-title {
  min-width: 0;
  line-height: 1.45;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.tool-call-batch-meta {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-secondary);
}

.tool-call-batch-chevron {
  position: absolute;
  left: 6px;
  top: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 12px;
  height: 12px;
  color: var(--text-secondary);
  transform: translateY(-50%);
  transition: transform 0.18s ease, color 0.15s;
}

.tool-call-batch-summary:hover .tool-call-batch-chevron,
.tool-call-batch-summary:focus-visible .tool-call-batch-chevron,
.tool-call-batch-summary.open .tool-call-batch-chevron {
  color: var(--text-color);
}

.tool-call-batch-chevron svg {
  display: block;
  overflow: visible;
}

.tool-call-batch-chevron.open {
  transform: translateY(-50%) rotate(90deg);
}

.tool-call-collection-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tool-call-collection-list.with-summary {
  margin-top: 0;
}

.tool-call-collection-list.with-summary.open {
  padding: 8px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  border-top: none;
  border-radius: 0 0 8px 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--msg-assistant-bg) 18%);
}
</style>

````

### src/components/ToolCallBlock.vue

行数：1068

````vue

<script setup lang="ts">
import { ref, computed, nextTick, watch } from "vue";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { canvasSetSpec } from "../services/canvas";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import ToolCallCollection from "./ToolCallCollection.vue";
import FileDiffViewer from "./diff/FileDiffViewer.vue";
import hljs, { langFromPath } from "../hljs";
import { diffStrings } from "../services/diff";
import { t } from "../i18n";
import { resolveToolBlockOverride } from "./tool-block-overrides/toolBlockOverrides";
import { buildToolCallArgsSummary } from "./toolCallSummary";

import type { ToolCallDisplay, FileDiffPayload } from "../types";

const props = withDefaults(defineProps<{
  toolCall: ToolCallDisplay;
  collapseEnabled?: boolean;
}>(), {
  collapseEnabled: true,
});
const emit = defineEmits<{
  (e: "toolViewportAnchorStart", anchor: HTMLElement): void;
  (e: "toolViewportAnchorEnd", anchor: HTMLElement): void;
}>();

function isSubagentToolName(name: string) {
  return name === "explore" || name === "task";
}

function shouldAutoExpandSubagentTool(toolCall: ToolCallDisplay) {
  return isSubagentToolName(toolCall.name) && toolCall.status === "running";
}

const expanded = ref(shouldAutoExpandSubagentTool(props.toolCall));
const rootRef = ref<HTMLElement | null>(null);
const headerRef = ref<HTMLElement | null>(null);
const outputPre = ref<HTMLPreElement | null>(null);

watch(
  () => [props.toolCall.output, props.toolCall.nestedToolCalls?.length],
  () => {
    if (outputPre.value && props.toolCall.status === "running") {
      nextTick(() => {
        if (outputPre.value) {
          outputPre.value.scrollTop = outputPre.value.scrollHeight;
        }
      });
    }
  }
);

const isSubagentTool = computed(() => {
  const name = props.toolCall.name;
  return isSubagentToolName(name);
});

const waitingLabel = computed(() => (
  isSubagentTool.value ? t("tool.subagentWaiting") : t("tool.waiting")
));

const isCanvasTool = computed(() => props.toolCall.name === "canvas");
const showRecompileHint = computed(() => props.toolCall.name === "unity_recompile" && props.toolCall.status === "running");
const toolBlockOverride = computed(() => resolveToolBlockOverride(props.toolCall.name));

function runOnNextFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

function emitToolViewportAnchorStart(anchor: HTMLElement) {
  emit("toolViewportAnchorStart", anchor);
}

function emitToolViewportAnchorEnd(anchor: HTMLElement) {
  emit("toolViewportAnchorEnd", anchor);
}

function setExpanded(nextExpanded: boolean, preserveViewport = false) {
  if (expanded.value === nextExpanded) return;
  const anchor = headerRef.value ?? rootRef.value;
  if (preserveViewport && anchor) emitToolViewportAnchorStart(anchor);
  expanded.value = nextExpanded;

  if (preserveViewport && anchor) {
    nextTick(() => {
      runOnNextFrame(() => emitToolViewportAnchorEnd(anchor));
    });
  }
}

function toggleExpanded() {
  setExpanded(!expanded.value, true);
}

watch(
  () => [props.toolCall.id, props.toolCall.name, props.toolCall.status] as const,
  ([nextId, _nextName, nextStatus], [previousId, _previousName, previousStatus]) => {
    if (nextId !== previousId) {
      setExpanded(shouldAutoExpandSubagentTool(props.toolCall), false);
      return;
    }
    if (!isSubagentTool.value) return;
    if (previousStatus === "running" && nextStatus !== "running") {
      setExpanded(false, true);
    } else if (previousStatus !== "running" && nextStatus === "running") {
      setExpanded(true, true);
    }
  },
);

const canvasInfo = computed(() => {
  if (!isCanvasTool.value) return null;
  try {
    const args = JSON.parse(props.toolCall.arguments);
    const spec = args.spec;
    if (!spec) return null;
    return {
      title: spec.title || "Canvas",
      nodeCount: spec.nodes?.length || 0,
      edgeCount: spec.edges?.length || 0,
    };
  } catch {
    return null;
  }
});

async function openCanvasWindow() {
  try {
    const args = JSON.parse(props.toolCall.arguments);
    const spec = args.spec;
    if (!spec) return;

    const specId = props.toolCall.id;

    const existingWin = await WebviewWindow.getByLabel(`canvas-${specId}`);
    if (existingWin) {
      await existingWin.setFocus();
      return;
    }

    await canvasSetSpec(specId, JSON.stringify(spec));

    const canvasWin = new WebviewWindow(`canvas-${specId}`, {
      url: `/canvas?specId=${specId}`,
      title: `Canvas: ${spec.title || "Canvas"}`,
      width: 1200,
      height: 800,
      minWidth: 600,
      minHeight: 400,
      decorations: true,
      resizable: true,
      center: true,
    });

    canvasWin.once("tauri://error", (e) => {
      console.error("Canvas window error:", e);
    });
  } catch (e) {
    console.error("Failed to open canvas window:", e);
  }
}

const statusIcon = computed(() => {
  switch (props.toolCall.status) {
    case "running": return "spinner";
    case "done": return "check";
    case "error": return "error";
    case "interrupted": return "error";
  }
});

const displayName = computed(() => {
  if (props.toolCall.name === "task") {
    try {
      const args = JSON.parse(props.toolCall.arguments);
      return args.subagent_type || "task";
    } catch {
      return "task";
    }
  }
  return props.toolCall.name;
});

const isEditTool = computed(() => props.toolCall.name === "edit");

interface EditDiffItem {
  oldStr: string;
  newStr: string;
  replaceAll: boolean;
  startLine: number;
}
interface EditDiffResult {
  filePath: string;
  items: EditDiffItem[];
}
function parseEditStartLines(output: string | undefined): number[] {
  if (!output) return [];
  const m = output.match(/\[lines:([0-9,]+)\]/);
  if (!m) return [];
  return m[1].split(",").map(Number);
}

const editDiffData = computed((): EditDiffResult | null => {
  if (!isEditTool.value) return null;
  try {
    const args = JSON.parse(props.toolCall.arguments);
    const filePath = args.filePath || args.file_path || args.path || "";
    const startLines = parseEditStartLines(props.toolCall.output);
    const items: EditDiffItem[] = [];
    if (Array.isArray(args.edits)) {
      for (let i = 0; i < args.edits.length; i++) {
        const edit = args.edits[i];
        items.push({
          oldStr: edit.oldString || edit.old_string || "",
          newStr: edit.newString || edit.new_string || "",
          replaceAll: edit.replaceAll || edit.replace_all || false,
          startLine: startLines[i] || 0,
        });
      }
    } else {
      const oldStr = args.oldString || args.old_string || "";
      const newStr = args.newString || args.new_string || "";
      if (oldStr || newStr) {
        items.push({
          oldStr,
          newStr,
          replaceAll: args.replaceAll || args.replace_all || false,
          startLine: startLines[0] || 0,
        });
      }
    }
    if (items.length === 0) return null;
    return { filePath, items };
  } catch {
    return null;
  }
});

// Compute diff payloads for each edit item using backend diff_strings
const editDiffPayloads = ref<Map<number, FileDiffPayload>>(new Map());

watch(editDiffData, async (data) => {
  editDiffPayloads.value = new Map();
  if (!data) return;
  for (let i = 0; i < data.items.length; i++) {
    const item = data.items[i];
    try {
      const hunks = await diffStrings(item.oldStr, item.newStr, 3);
      const additions = hunks.reduce((sum, h) => sum + h.lines.filter(l => l.kind === "add").length, 0);
      const deletions = hunks.reduce((sum, h) => sum + h.lines.filter(l => l.kind === "delete").length, 0);
      const payload: FileDiffPayload = {
        key: `edit-${i}`,
        filePath: data.filePath,
        status: "M",
        language: langFromPath(data.filePath) || undefined,
        isBinary: false,
        isLarge: false,
        contentState: { type: 'normal' },
        stats: { additions, deletions, changedHunks: hunks.length },
        previewSummary: [`+${additions} -${deletions}`],
        text: { hunks },
      };
      editDiffPayloads.value.set(i, payload);
    } catch {
      // Fall through to old rendering if diff fails
    }
  }
}, { immediate: true });

/** Syntax-highlight diff content and return HTML with line numbers.
 *  startLine: 1-based line number in the source file, 0 means start from 1
 */
function highlightDiffCode(code: string, filePath: string, startLine: number): string {
  if (!code) return "";
  const lang = filePath ? langFromPath(filePath) : null;
  let highlighted: string;
  if (lang) {
    try {
      highlighted = hljs.highlight(code, { language: lang }).value;
    } catch {
      highlighted = escapeHtml(code);
    }
  } else {
    highlighted = escapeHtml(code);
  }
  const base = startLine > 0 ? startLine : 1;
  const lines = highlighted.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((line, i) =>
    `<div class="edit-diff-line"><span class="edit-diff-ln">${base + i}</span><span class="edit-diff-line-content">${line || " "}</span></div>`
  ).join("");
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

const parsedArgs = computed(() => {
  try {
    const args = JSON.parse(props.toolCall.arguments);
    if (typeof args !== "object" || args === null) return [];
    const isTask = props.toolCall.name === "task";
    const isEdit = props.toolCall.name === "edit";
    const editDiffKeys = ["oldString", "old_string", "newString", "new_string", "edits"];
    return Object.entries(args)
      .filter(([key]) => !isTask || key === "prompt")
      .filter(([key]) => !isEdit || !editDiffKeys.includes(key))
      .map(([key, value]) => ({
        key,
        value,
        isLong: typeof value === "string" && (value as string).length > 80,
        isMultiline: typeof value === "string" && (value as string).includes("\n"),
      }));
  } catch {
    return [];
  }
});

const rawArgsFallback = computed(() => {
  if (parsedArgs.value.length > 0) return "";
  return props.toolCall.arguments;
});

function formatValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return String(value);
  if (value === null) return "null";
  return JSON.stringify(value, null, 2);
}

function prettifyKey(key: string): string {
  return key
    .replace(/_/g, " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .toLowerCase();
}

const argsSummary = computed(() =>
  buildToolCallArgsSummary(props.toolCall.name, props.toolCall.arguments),
);

function getFilePath(): string {
  try {
    const args = JSON.parse(props.toolCall.arguments);
    return args.filePath || args.file_path || args.path || "";
  } catch {
    return "";
  }
}

function unwrapPersistedOutput(output: string): string {
  const match = output.match(/^<persisted-output>\n?([\s\S]*?)\n?<\/persisted-output>\s*$/);
  return match ? match[1].trim() : output;
}

const displayOutput = computed(() => {
  const output = props.toolCall.output;
  return output ? unwrapPersistedOutput(output) : "";
});

const highlightedOutput = computed(() => {
  const output = props.toolCall.output;
  if (!output) return null;
  const name = props.toolCall.name;
  if (name !== "read" && name !== "write" && name !== "edit") return null;
  const filePath = getFilePath();
  if (!filePath) return null;
  const lang = langFromPath(filePath);
  if (!lang) return null;
  try {
    let code = output;
    const contentMatch = code.match(/^<content>\n?([\s\S]*?)\n?<\/content>\s*$/);
    if (contentMatch) {
      code = contentMatch[1];
    }
    return hljs.highlight(code, { language: lang }).value;
  } catch {
    return null;
  }
});


</script>

<template>
  <component
    :is="toolBlockOverride"
    v-if="toolBlockOverride"
    :tool-call="toolCall"
    :collapse-enabled="collapseEnabled"
    @tool-viewport-anchor-start="emitToolViewportAnchorStart"
    @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
  />
  <div
    v-else
    ref="rootRef"
    class="tool-call-block"
    :class="[toolCall.status, { 'is-expanded': expanded, 'is-recompile-attention': showRecompileHint }]"
  >
    <button ref="headerRef" type="button" class="tool-call-header ui-select-none" @click="toggleExpanded">
      <span class="tool-call-icon" :class="statusIcon">
        <span v-if="toolCall.status === 'running'" class="spinner-anim"></span>
        <span v-else class="tool-call-status-dot"></span>
      </span>
      <span class="tool-call-name">{{ displayName }}</span>
      <span v-if="argsSummary" class="tool-call-summary">{{ argsSummary }}</span>
    </button>
    <div v-if="showRecompileHint" class="recompile-hint">
      <div class="recompile-hint-main">{{ t("tool.recompile.hint") }}</div>
      <div class="recompile-hint-sub">{{ t("tool.recompile.sub") }}</div>
    </div>
    <div v-if="isCanvasTool && canvasInfo && toolCall.status === 'done'" class="canvas-tool-summary">
      <button class="canvas-open-btn" @click.stop="openCanvasWindow">
        {{ t("tool.canvas.open") }}
      </button>
    </div>
    <div v-if="expanded" class="tool-call-detail">
      <div class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.section.args") }}</div>
        <template v-if="isEditTool && editDiffData">
          <div v-if="parsedArgs.length > 0" class="tool-args-table" style="margin-bottom: 6px;">
            <div v-for="arg in parsedArgs" :key="arg.key" class="tool-arg-row" :class="{ 'arg-block': arg.isMultiline || arg.isLong }">
              <span class="tool-arg-key">{{ prettifyKey(arg.key) }}</span>
              <pre v-if="arg.isMultiline" class="tool-arg-value-block">{{ formatValue(arg.value) }}</pre>
              <span v-else class="tool-arg-value" :class="{ 'value-bool': typeof arg.value === 'boolean', 'value-num': typeof arg.value === 'number' }">{{ formatValue(arg.value) }}</span>
            </div>
          </div>
          <template v-for="(item, idx) in editDiffData.items" :key="idx">
            <FileDiffViewer
              v-if="editDiffPayloads.get(idx)"
              :payload="editDiffPayloads.get(idx)!"
              :style="idx > 0 ? 'margin-top: 6px' : ''"
            />
            <div v-else class="edit-diff-container" :style="idx > 0 ? 'margin-top: 6px' : ''">
              <div class="edit-diff-panel edit-diff-old">
                <div class="edit-diff-panel-header edit-diff-header-old">
                  <span class="edit-diff-indicator">&#x2212;</span>
                  <span>{{ t("tool.diff.old") }}{{ editDiffData.items.length > 1 ? ` #${idx + 1}` : '' }}</span>
                </div>
                <pre class="edit-diff-code hljs" v-html="highlightDiffCode(item.oldStr, editDiffData.filePath, item.startLine)"></pre>
              </div>
              <div class="edit-diff-panel edit-diff-new">
                <div class="edit-diff-panel-header edit-diff-header-new">
                  <span class="edit-diff-indicator">&#x2b;</span>
                  <span>{{ t("tool.diff.new") }}{{ editDiffData.items.length > 1 ? ` #${idx + 1}` : '' }}</span>
                </div>
                <pre class="edit-diff-code hljs" v-html="highlightDiffCode(item.newStr, editDiffData.filePath, item.startLine)"></pre>
              </div>
            </div>
          </template>
        </template>
        <template v-else-if="isSubagentTool && parsedArgs.length === 1 && parsedArgs[0].key === 'prompt'">
          <pre class="tool-arg-prompt-direct ui-select-text">{{ formatValue(parsedArgs[0].value) }}</pre>
        </template>
        <div v-else-if="parsedArgs.length > 0" class="tool-args-table">
          <div v-for="arg in parsedArgs" :key="arg.key" class="tool-arg-row" :class="{ 'arg-block': arg.isMultiline || arg.isLong }">
            <span class="tool-arg-key">{{ prettifyKey(arg.key) }}</span>
              <pre v-if="arg.isMultiline" class="tool-arg-value-block ui-select-text">{{ formatValue(arg.value) }}</pre>
            <span v-else class="tool-arg-value" :class="{ 'value-bool': typeof arg.value === 'boolean', 'value-num': typeof arg.value === 'number' }">{{ formatValue(arg.value) }}</span>
          </div>
        </div>
        <pre v-else-if="rawArgsFallback" class="tool-call-pre ui-select-text">{{ rawArgsFallback }}</pre>
      </div>
      <div v-if="toolCall.output !== undefined || toolCall.status === 'running'" class="tool-call-section">
        <div class="tool-call-section-label">
          {{ t("tool.section.output") }}
          <span v-if="toolCall.status === 'running' && toolCall.output" class="output-streaming-indicator"></span>
        </div>
        <template v-if="toolCall.output || (isSubagentTool && toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0)">
          <div v-if="isSubagentTool && toolCall.status !== 'error'" class="subagent-output ui-select-text" :class="{ 'streaming-output': toolCall.status === 'running' }" ref="outputPre">
            <div v-if="toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0" class="nested-tool-calls">
              <ToolCallCollection
                :tool-calls="toolCall.nestedToolCalls"
                :collapse-enabled="collapseEnabled"
                @viewport-anchor-start="emitToolViewportAnchorStart"
                @viewport-anchor-end="emitToolViewportAnchorEnd"
              >
                <template #default="{ toolCall: nestedToolCall }">
                  <ToolCallBlock
                    :tool-call="nestedToolCall"
                    :collapse-enabled="collapseEnabled"
                    @tool-viewport-anchor-start="emitToolViewportAnchorStart"
                    @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
                  />
                </template>
              </ToolCallCollection>
            </div>
            <MarkdownRenderer v-if="toolCall.output" :content="toolCall.output" />
          </div>
          <pre v-else-if="toolCall.output && highlightedOutput" class="tool-call-pre ui-select-text hljs" :class="{ 'error-output': toolCall.status === 'error', 'streaming-output': toolCall.status === 'running' }" ref="outputPre" v-html="highlightedOutput"></pre>
          <pre v-else-if="toolCall.output" class="tool-call-pre ui-select-text" :class="{ 'error-output': toolCall.status === 'error', 'streaming-output': toolCall.status === 'running' }" ref="outputPre">{{ displayOutput }}</pre>
        </template>
        <template v-else>
          <div v-if="toolCall.status === 'running'" class="tool-call-waiting">
            <span class="waiting-dots"></span>
            <span class="waiting-text">{{ waitingLabel }}</span>
          </div>
          <pre v-else class="tool-call-pre ui-select-text">{{ t("tool.noOutput") }}</pre>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tool-call-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  max-width: 100%;
  margin: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  overflow: visible;
  font-size: 13px;
}

.tool-call-block.is-expanded {
  width: 100%;
}

.tool-call-block.is-recompile-attention {
  align-items: stretch;
  padding: 4px 6px 6px;
  border: 1px solid var(--status-warn-border);
  border-left-width: 3px;
  border-left-color: var(--status-warn-fg);
  border-radius: 4px;
  background: color-mix(in srgb, var(--status-warn-bg) 82%, var(--panel-bg) 18%);
  overflow: hidden;
}

.tool-call-header {
  appearance: none;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  width: 100%;
  max-width: 100%;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 1px 4px;
  border-radius: 4px;
  cursor: pointer;
  user-select: none;
  min-height: 22px;
  text-align: left;
  transition: color 0.12s ease, background 0.12s ease;
}

.tool-call-header:hover {
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
}

.tool-call-header:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--accent-color) 36%, transparent);
  outline-offset: 1px;
}

.tool-call-block.is-recompile-attention .tool-call-icon.spinner {
  color: var(--status-warn-fg);
}

.tool-call-icon {
  width: 14px;
  height: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.tool-call-icon.spinner {
  color: var(--accent-color);
}

.tool-call-icon.check {
  color: var(--text-secondary);
}

.tool-call-icon.error {
  color: var(--status-danger-fg);
}

.tool-call-status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
  opacity: 0.7;
}

.tool-call-icon.check .tool-call-status-dot {
  opacity: 0.46;
}

.tool-call-icon.error .tool-call-status-dot {
  width: 6px;
  height: 6px;
  opacity: 0.78;
}

.spinner-anim {
  width: 10px;
  height: 10px;
  border: 1.5px solid color-mix(in srgb, var(--accent-color) 18%, transparent);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: tool-spin 0.8s linear infinite;
  display: inline-block;
}

@keyframes tool-spin {
  to { transform: rotate(360deg); }
}

.tool-call-name {
  font-weight: 600;
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
  font-size: 12px;
  flex-shrink: 0;
}

.tool-call-summary {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.tool-call-detail {
  align-self: stretch;
  margin-top: 4px;
  padding: 6px 0 0 26px;
}

.tool-call-section {
  margin-bottom: 6px;
}

.tool-call-section:last-child {
  margin-bottom: 0;
}

.tool-call-section-label {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  margin-bottom: 4px;
}

.tool-args-table {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
}

.tool-arg-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  line-height: 1.5;
  font-size: 12px;
}

.tool-arg-row.arg-block {
  flex-direction: column;
  gap: 2px;
}

.tool-arg-key {
  color: var(--text-secondary);
  font-size: 11px;
  flex-shrink: 0;
  min-width: 60px;
  font-weight: 500;
}

.tool-arg-value {
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-color);
  word-break: break-word;
  min-width: 0;
}

.tool-arg-value.value-bool {
  color: #d2a8ff;
}

.tool-arg-value.value-num {
  color: #79c0ff;
}

.tool-arg-value-block {
  font-family: var(--font-mono-block);
  font-size: 12px;
  color: var(--text-color);
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  padding: 4px 6px;
  border-radius: 4px;
  background: rgba(0, 0, 0, 0.15);
  line-height: 1.4;
}

.tool-arg-prompt-direct {
  font-family: var(--font-mono-block);
  font-size: 12px;
  color: var(--text-color);
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  line-height: 1.5;
}

.tool-call-pre {
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.4;
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
}

.subagent-output {
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  overflow-y: auto;
  max-height: 600px;
  scrollbar-gutter: stable;
}

.nested-tool-calls {
  margin-bottom: 4px;
}

.nested-tool-calls :deep(.tool-call-collection) {
  gap: 2px;
}

.nested-tool-calls :deep(.tool-call-collection-list) {
  gap: 2px;
}

.nested-tool-calls :deep(.tool-call-block) {
  border-color: rgba(128, 128, 128, 0.2);
  font-size: 12px;
}

.nested-tool-calls :deep(.tool-call-header) {
  gap: 5px;
  min-height: 18px;
  padding: 0 4px;
  border-radius: 3px;
}

.nested-tool-calls :deep(.tool-call-icon) {
  width: 12px;
  height: 12px;
}

.nested-tool-calls :deep(.tool-call-status-dot) {
  width: 4px;
  height: 4px;
}

.nested-tool-calls :deep(.spinner-anim) {
  width: 8px;
  height: 8px;
  border-width: 1px;
}

.nested-tool-calls :deep(.tool-call-name) {
  font-size: 11px;
}

.nested-tool-calls :deep(.tool-call-summary) {
  font-size: 10px;
}

.nested-tool-calls :deep(.tool-call-status) {
  font-size: 10px;
}

.nested-tool-calls :deep(.tool-call-detail) {
  margin-top: 2px;
  padding: 3px 0 0 18px;
}

.nested-tool-calls :deep(.tool-call-batch-summary) {
  min-height: 20px;
  padding: 1px 6px 1px 15px;
  border-color: transparent;
  border-radius: 4px;
}

.nested-tool-calls :deep(.tool-call-batch-summary:hover),
.nested-tool-calls :deep(.tool-call-batch-summary:focus-visible) {
  border-color: rgba(128, 128, 128, 0.2);
}

.nested-tool-calls :deep(.tool-call-batch-summary.open) {
  border-color: rgba(128, 128, 128, 0.24);
  border-radius: 6px 6px 0 0;
}

.nested-tool-calls :deep(.tool-call-batch-summary.open.closing) {
  border-color: transparent;
  border-radius: 6px;
}

.nested-tool-calls :deep(.tool-call-batch-chevron) {
  left: 3px;
  width: 10px;
  height: 10px;
}

.nested-tool-calls :deep(.tool-call-batch-chevron svg) {
  width: 9px;
  height: 9px;
}

.nested-tool-calls :deep(.tool-call-batch-title) {
  font-size: 11px;
}

.nested-tool-calls :deep(.tool-call-batch-meta) {
  font-size: 10px;
}

.nested-tool-calls :deep(.tool-call-collection-list.with-summary.open) {
  padding: 4px;
  border-color: rgba(128, 128, 128, 0.24);
  border-radius: 0 0 6px 6px;
}

.error-output {
  color: #f85149;
}

.streaming-output {
  max-height: 300px;
  overflow-y: auto;
  scrollbar-gutter: stable;
  border-left: 2px solid #4a9eff;
}

.output-streaming-indicator {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #4a9eff;
  margin-left: 6px;
  vertical-align: middle;
  animation: output-pulse 1s ease-in-out infinite;
}

@keyframes output-pulse {
  0%, 100% { opacity: 0.3; }
  50% { opacity: 1; }
}

.tool-call-waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  font-size: 12px;
  color: var(--text-secondary);
  scrollbar-gutter: stable;
}

.waiting-dots {
  display: inline-flex;
  gap: 3px;
}

.waiting-dots::before,
.waiting-dots::after {
  content: "";
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--text-secondary);
  animation: dot-bounce 1.4s infinite ease-in-out both;
}

.waiting-dots::before {
  animation-delay: 0s;
}

.waiting-dots::after {
  animation-delay: 0.32s;
}

@keyframes dot-bounce {
  0%, 80%, 100% { opacity: 0.2; transform: scale(0.8); }
  40% { opacity: 1; transform: scale(1); }
}

.waiting-text {
  font-style: italic;
}

.recompile-hint {
  align-self: stretch;
  margin-top: 4px;
  padding: 6px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--status-warn-border) 72%, transparent);
}

.recompile-hint-main {
  font-size: 13px;
  font-weight: 600;
  color: var(--status-warn-fg);
}

.recompile-hint-sub {
  font-size: 11px;
  color: color-mix(in srgb, var(--status-warn-fg) 48%, var(--text-secondary));
  margin-top: 2px;
}

.canvas-tool-summary {
  padding: 6px 10px;
  border-top: 1px solid var(--border-color);
}

.canvas-open-btn {
  background: #2d5a3e;
  border: 1px solid #3fb950;
  color: #3fb950;
  padding: 5px 16px;
  border-radius: 5px;
  cursor: pointer;
  font-size: 12px;
  font-weight: 500;
  transition: background 0.15s;
}

.canvas-open-btn:hover {
  background: #3a6b4e;
  color: #fff;
}

.edit-diff-container {
  display: flex;
  flex-direction: column;
  gap: 0;
  border-radius: 6px;
  overflow: hidden;
  border: 1px solid var(--border-color);
  background: var(--hover-bg);
}

.edit-diff-panel {
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
}

.edit-diff-panel-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  user-select: none;
  flex-shrink: 0;
}

.edit-diff-header-old {
  background: rgba(248, 81, 73, 0.15);
  color: #f85149;
  border-bottom: 1px solid rgba(248, 81, 73, 0.15);
}

.edit-diff-header-new {
  background: rgba(63, 185, 80, 0.15);
  color: #3fb950;
  border-bottom: 1px solid rgba(63, 185, 80, 0.15);
}

.edit-diff-indicator {
  font-size: 14px;
  font-weight: 700;
  line-height: 1;
}

.edit-diff-code {
  font-family: var(--font-mono-block);
  font-size: 13px;
  line-height: 1.6;
  padding: 12px 0;
  margin: 0;
  white-space: pre;
  overflow-x: auto;
  flex: 1;
  min-height: 0;
}

.edit-diff-code :deep(.edit-diff-line) {
  display: block;
}

.edit-diff-code :deep(.edit-diff-ln) {
  display: inline-block;
  width: 3.5em;
  padding-right: 12px;
  text-align: right;
  color: var(--line-number-color, #6e7681);
  user-select: none;
  opacity: 0.6;
  font-size: 12px;
  font-family: inherit;
}

.edit-diff-code :deep(.edit-diff-line-content) {
  padding-left: 4px;
}

.edit-diff-old .edit-diff-code {
  border-left: 3px solid rgba(248, 81, 73, 0.6);
}

.edit-diff-new .edit-diff-code {
  border-left: 3px solid rgba(63, 185, 80, 0.6);
}

.edit-diff-old {
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}
</style>

````

### src/composables/toolCallBatches.ts

行数：684

````ts
import type { AssistantRenderPart, ChatMessage, ToolCallDisplay, ToolCallInfo } from "../types";

const INTERRUPTED_TOOL_RESULT = "工具执行被用户中止，未返回结果。";
const GENERIC_ARGUMENT_ALIAS_GROUPS: Array<readonly [string, readonly string[]]> = [
  ["filePath", ["filePath", "file_path"]],
  ["oldString", ["oldString", "old_string"]],
  ["newString", ["newString", "new_string"]],
  ["replaceAll", ["replaceAll", "replace_all"]],
  ["editorStatus", ["editorStatus", "editor_status"]],
  ["requestEditorStatus", ["requestEditorStatus", "request_editor_status"]],
  ["assetPath", ["assetPath", "asset_path"]],
  ["maxDepth", ["maxDepth", "max_depth"]],
  ["typeFilter", ["typeFilter", "type_filter"]],
  ["objectPath", ["objectPath", "object_path"]],
  ["includeFiles", ["includeFiles", "include_files"]],
  ["maxItems", ["maxItems", "max_items"]],
  ["maxTotal", ["maxTotal", "max_total"]],
  ["scenePath", ["scenePath", "scene_path"]],
  ["sourceField", ["sourceField", "source_field"]],
  ["subagentType", ["subagentType", "subagent_type"]],
];

const TOOL_SPECIFIC_ARGUMENT_ALIAS_GROUPS: Partial<
  Record<string, Array<readonly [string, readonly string[]]>>
> = {
  read: [["filePath", ["filePath", "file_path", "path"]]],
  write: [["filePath", ["filePath", "file_path", "path"]]],
  edit: [["filePath", ["filePath", "file_path", "path"]]],
  list: [["path", ["path", "filePath", "file_path"]]],
  grep: [["path", ["path", "filePath", "file_path"]]],
};

const PATH_LIKE_ARGUMENT_KEYS = new Set([
  "filePath",
  "path",
  "assetPath",
  "objectPath",
  "scenePath",
]);

export interface ToolCallMatchState {
  ids: Set<string>;
  fingerprintCounts: Map<string, number>;
  idFingerprints: Map<string, string>;
}

export interface ToolCallBatchState {
  total: number;
  doneCount: number;
  runningCount: number;
  errorCount: number;
  interruptedCount: number;
  canCollapse: boolean;
}

export interface AssistantToolMergeCandidate {
  id: string;
  content: string;
  thinkingContent?: string;
  renderParts?: unknown[];
  toolCalls?: ToolCallInfo[];
  attachedKnowledgeProposalCount?: number;
  isKnowledgeProposal?: boolean;
}

export type AssistantToolMergeResult<T> = T & {
  displayToolCalls?: ToolCallInfo[];
  displayToolCallsBeforeContent?: ToolCallInfo[];
  displayToolCallsAfterContent?: ToolCallInfo[];
};

export interface ToolCallInfoRenderSource {
  messageToolCalls?: ToolCallInfo[];
  displayToolCalls?: ToolCallInfo[];
}

interface OrderedToolCallLike {
  order?: number;
  nestedToolCalls?: readonly OrderedToolCallLike[];
}

export interface ToolCallRenderOrderSegment<T> {
  order: number;
  toolCalls: T[];
}

export function firstToolCallRenderOrder(toolCalls: readonly OrderedToolCallLike[]) {
  let order = Number.POSITIVE_INFINITY;
  const visit = (items: readonly OrderedToolCallLike[]) => {
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

export function lastToolCallRenderOrder(toolCalls: readonly OrderedToolCallLike[]) {
  let order = 0;
  const visit = (items: readonly OrderedToolCallLike[]) => {
    for (const toolCall of items) {
      if (typeof toolCall.order === "number" && toolCall.order > 0) {
        order = Math.max(order, toolCall.order);
      }
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };
  visit(toolCalls);
  return order;
}

export function hasVisibleTextPartAfterToolCalls(
  parts: readonly AssistantRenderPart[],
  toolCalls: readonly OrderedToolCallLike[],
) {
  const lastToolOrder = lastToolCallRenderOrder(toolCalls);
  if (lastToolOrder <= 0) return false;
  return parts.some((part) =>
    part.kind === "text"
    && part.content.trim().length > 0
    && part.order.seq > lastToolOrder,
  );
}

export function splitToolCallsByRenderOrder<T extends OrderedToolCallLike>(
  toolCalls: readonly T[],
  options: { fallbackOrder: number; boundaryOrders?: readonly number[] },
): Array<ToolCallRenderOrderSegment<T>> {
  const boundaryOrders = [...(options.boundaryOrders ?? [])]
    .filter((order) => Number.isFinite(order) && order > 0)
    .sort((left, right) => left - right);
  const entries = toolCalls
    .map((toolCall, index) => ({
      toolCall,
      index,
      order: firstToolCallRenderOrder([toolCall]) || options.fallbackOrder,
    }))
    .sort((left, right) => left.order - right.order || left.index - right.index);
  const segments: Array<ToolCallRenderOrderSegment<T>> = [];

  const hasBoundaryBetween = (leftOrder: number, rightOrder: number) =>
    boundaryOrders.some((boundaryOrder) =>
      boundaryOrder > leftOrder && boundaryOrder <= rightOrder,
    );

  for (const entry of entries) {
    const current = segments[segments.length - 1];
    if (!current || hasBoundaryBetween(current.order, entry.order)) {
      segments.push({ order: entry.order, toolCalls: [entry.toolCall] });
      continue;
    }
    current.toolCalls.push(entry.toolCall);
  }

  return segments;
}

function stableSerialize(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableSerialize(item)).join(",")}]`;
  }
  if (value && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, nestedValue]) => `${JSON.stringify(key)}:${stableSerialize(nestedValue)}`);
    return `{${entries.join(",")}}`;
  }
  return JSON.stringify(value);
}

function normalizePathLikeArgument(value: string): string {
  return value.replace(/\\/g, "/");
}

function normalizeArgumentValue(key: string, value: unknown): unknown {
  if (typeof value === "string" && PATH_LIKE_ARGUMENT_KEYS.has(key)) {
    return normalizePathLikeArgument(value);
  }
  return value;
}

function resolveAliasValue(
  source: Record<string, unknown>,
  aliases: readonly string[],
): unknown {
  for (const alias of aliases) {
    const value = source[alias];
    if (value !== undefined) {
      return value;
    }
  }
  return undefined;
}

function canonicalizeArgumentObject(
  toolName: string,
  value: Record<string, unknown>,
  isRoot: boolean,
): Record<string, unknown> {
  const canonical: Record<string, unknown> = {};

  for (const [key, nestedValue] of Object.entries(value)) {
    canonical[key] = canonicalizeArgumentValue(toolName, nestedValue, false);
  }

  const aliasGroups = [
    ...GENERIC_ARGUMENT_ALIAS_GROUPS,
    ...(isRoot ? (TOOL_SPECIFIC_ARGUMENT_ALIAS_GROUPS[toolName] ?? []) : []),
  ];

  for (const [canonicalKey, aliases] of aliasGroups) {
    const resolved = resolveAliasValue(canonical, aliases);
    for (const alias of aliases) {
      delete canonical[alias];
    }
    if (resolved !== undefined) {
      canonical[canonicalKey] = normalizeArgumentValue(canonicalKey, resolved);
    }
  }

  const normalized: Record<string, unknown> = {};
  for (const [key, nestedValue] of Object.entries(canonical)) {
    normalized[key] = normalizeArgumentValue(key, nestedValue);
  }
  return normalized;
}

function canonicalizeArgumentValue(
  toolName: string,
  value: unknown,
  isRoot: boolean,
): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => canonicalizeArgumentValue(toolName, item, false));
  }
  if (value && typeof value === "object") {
    return canonicalizeArgumentObject(toolName, value as Record<string, unknown>, isRoot);
  }
  return value;
}

function normalizeToolCallArguments(argumentsText: string): string {
  try {
    return stableSerialize(canonicalizeArgumentValue("", JSON.parse(argumentsText), true));
  } catch {
    return argumentsText.trim();
  }
}

export function getToolCallInfoFingerprint(toolCall: Pick<ToolCallInfo, "name" | "arguments" | "nestedToolCalls">): string {
  const nestedFingerprints = toolCall.nestedToolCalls?.map((nestedToolCall) => getToolCallInfoFingerprint(nestedToolCall)) ?? [];
  return `${toolCall.name}\u241f${normalizeToolCallArgumentsForTool(toolCall.name, toolCall.arguments)}\u241f${nestedFingerprints.join("\u241e")}`;
}

export function getToolCallDisplayFingerprint(toolCall: Pick<ToolCallDisplay, "name" | "arguments" | "nestedToolCalls">): string {
  const nestedFingerprints =
    toolCall.nestedToolCalls?.map((nestedToolCall) => getToolCallDisplayFingerprint(nestedToolCall)) ?? [];
  return `${toolCall.name}\u241f${normalizeToolCallArgumentsForTool(toolCall.name, toolCall.arguments)}\u241f${nestedFingerprints.join("\u241e")}`;
}

function normalizeToolCallArgumentsForTool(toolName: string, argumentsText: string): string {
  try {
    return stableSerialize(canonicalizeArgumentValue(toolName, JSON.parse(argumentsText), true));
  } catch {
    return normalizeToolCallArguments(argumentsText);
  }
}

function incrementCount(map: Map<string, number>, key: string) {
  map.set(key, (map.get(key) ?? 0) + 1);
}

export function collectToolCallDisplayIds(toolCalls: ToolCallDisplay[]): Set<string> {
  const ids = new Set<string>();

  const visit = (items: ToolCallDisplay[]) => {
    for (const toolCall of items) {
      ids.add(toolCall.id);
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };

  visit(toolCalls);
  return ids;
}

export function collectToolCallDisplayMatchState(toolCalls: ToolCallDisplay[]): ToolCallMatchState {
  const ids = new Set<string>();
  const fingerprintCounts = new Map<string, number>();
  const idFingerprints = new Map<string, string>();

  const visit = (items: ToolCallDisplay[]) => {
    for (const toolCall of items) {
      const fingerprint = getToolCallDisplayFingerprint(toolCall);
      ids.add(toolCall.id);
      idFingerprints.set(toolCall.id, fingerprint);
      incrementCount(fingerprintCounts, fingerprint);
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };

  visit(toolCalls);
  return { ids, fingerprintCounts, idFingerprints };
}

export function collectToolCallDisplayIdMatchState(toolCalls: ToolCallDisplay[]): ToolCallMatchState {
  return {
    ids: collectToolCallDisplayIds(toolCalls),
    fingerprintCounts: new Map<string, number>(),
    idFingerprints: new Map<string, string>(),
  };
}

export function mergeToolCallMatchStates(...states: ToolCallMatchState[]): ToolCallMatchState {
  const ids = new Set<string>();
  const fingerprintCounts = new Map<string, number>();
  const idFingerprints = new Map<string, string>();

  for (const state of states) {
    for (const id of state.ids) {
      ids.add(id);
    }
    for (const [id, fingerprint] of state.idFingerprints) {
      idFingerprints.set(id, fingerprint);
    }
    for (const [fingerprint, count] of state.fingerprintCounts) {
      fingerprintCounts.set(fingerprint, (fingerprintCounts.get(fingerprint) ?? 0) + count);
    }
  }

  return { ids, fingerprintCounts, idFingerprints };
}

export function cloneToolCallMatchState(state: ToolCallMatchState): ToolCallMatchState {
  return {
    ids: new Set(state.ids),
    fingerprintCounts: new Map(state.fingerprintCounts),
    idFingerprints: new Map(state.idFingerprints),
  };
}

export function areToolCallDisplaysCoveredByMatchState(
  toolCalls: ToolCallDisplay[],
  state: ToolCallMatchState,
): boolean {
  if (toolCalls.length === 0) return false;
  if (state.ids.size === 0 && state.fingerprintCounts.size === 0) return false;

  const remainingState = cloneToolCallMatchState(state);
  return toolCalls.every((toolCall) => consumeDisplayMatch(toolCall, remainingState));
}

function consumeFingerprintMatch(state: ToolCallMatchState, fingerprint: string): boolean {
  const remaining = state.fingerprintCounts.get(fingerprint) ?? 0;
  if (remaining <= 0) return false;
  if (remaining === 1) {
    state.fingerprintCounts.delete(fingerprint);
  } else {
    state.fingerprintCounts.set(fingerprint, remaining - 1);
  }
  return true;
}

function consumeIdMatch(state: ToolCallMatchState, id: string, fallbackFingerprint: string): boolean {
  if (!state.ids.has(id)) return false;
  const fingerprint = state.idFingerprints.get(id) ?? fallbackFingerprint;
  state.ids.delete(id);
  state.idFingerprints.delete(id);
  consumeFingerprintMatch(state, fingerprint);
  return true;
}

function consumeFingerprintAndOneId(state: ToolCallMatchState, fingerprint: string): boolean {
  if (!consumeFingerprintMatch(state, fingerprint)) return false;
  for (const [id, storedFingerprint] of state.idFingerprints) {
    if (storedFingerprint !== fingerprint) continue;
    state.ids.delete(id);
    state.idFingerprints.delete(id);
    break;
  }
  return true;
}

function consumeInfoTreeMatchState(toolCall: ToolCallInfo, state: ToolCallMatchState) {
  const fingerprint = getToolCallInfoFingerprint(toolCall);
  if (!consumeIdMatch(state, toolCall.id, fingerprint)) {
    consumeFingerprintAndOneId(state, fingerprint);
  }
  for (const nestedToolCall of toolCall.nestedToolCalls ?? []) {
    consumeInfoTreeMatchState(nestedToolCall, state);
  }
}

function consumeDisplayTreeMatchState(toolCall: ToolCallDisplay, state: ToolCallMatchState) {
  const fingerprint = getToolCallDisplayFingerprint(toolCall);
  if (!consumeIdMatch(state, toolCall.id, fingerprint)) {
    consumeFingerprintAndOneId(state, fingerprint);
  }
  for (const nestedToolCall of toolCall.nestedToolCalls ?? []) {
    consumeDisplayTreeMatchState(nestedToolCall, state);
  }
}

function consumeInfoMatch(
  toolCall: ToolCallInfo,
  state: ToolCallMatchState,
): boolean {
  const fingerprint = getToolCallInfoFingerprint(toolCall);
  if (state.ids.has(toolCall.id) || (state.fingerprintCounts.get(fingerprint) ?? 0) > 0) {
    consumeInfoTreeMatchState(toolCall, state);
    return true;
  }
  return false;
}

function consumeDisplayMatch(
  toolCall: ToolCallDisplay,
  state: ToolCallMatchState,
): boolean {
  const fingerprint = getToolCallDisplayFingerprint(toolCall);
  if (state.ids.has(toolCall.id) || (state.fingerprintCounts.get(fingerprint) ?? 0) > 0) {
    consumeDisplayTreeMatchState(toolCall, state);
    return true;
  }
  return false;
}

function filterToolCallInfoArray(
  toolCalls: ToolCallInfo[],
  state: ToolCallMatchState,
): ToolCallInfo[] {
  const filtered: ToolCallInfo[] = [];
  for (const toolCall of toolCalls) {
    if (consumeInfoMatch(toolCall, state)) continue;
    const nestedToolCalls =
      toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0
        ? filterToolCallInfoArray(toolCall.nestedToolCalls, state)
        : toolCall.nestedToolCalls;
    filtered.push(
      nestedToolCalls !== toolCall.nestedToolCalls
        ? { ...toolCall, nestedToolCalls }
        : toolCall,
    );
  }
  return filtered;
}

export function filterToolCallsByActiveIds(
  toolCalls: ToolCallInfo[] | undefined,
  activeIds: Set<string>,
): ToolCallInfo[] | undefined {
  if (!toolCalls || toolCalls.length === 0) return undefined;
  if (activeIds.size === 0) return [...toolCalls];

  const filtered = toolCalls.filter((toolCall) => !activeIds.has(toolCall.id));
  return filtered.length > 0 ? filtered : undefined;
}

export function filterToolCallsByMatchState(
  toolCalls: ToolCallInfo[] | undefined,
  hiddenState: ToolCallMatchState,
): ToolCallInfo[] | undefined {
  if (!toolCalls || toolCalls.length === 0) return undefined;
  if (hiddenState.ids.size === 0 && hiddenState.fingerprintCounts.size === 0) return [...toolCalls];

  const filtered = filterToolCallInfoArray(toolCalls, cloneToolCallMatchState(hiddenState));
  return filtered.length > 0 ? filtered : undefined;
}

export function filterToolCallsByConsumableMatchState(
  toolCalls: ToolCallInfo[] | undefined,
  hiddenState: ToolCallMatchState,
): ToolCallInfo[] | undefined {
  if (!toolCalls || toolCalls.length === 0) return undefined;
  if (hiddenState.ids.size === 0 && hiddenState.fingerprintCounts.size === 0) return [...toolCalls];

  const filtered = filterToolCallInfoArray(toolCalls, hiddenState);
  return filtered.length > 0 ? filtered : undefined;
}

export function mergeToolCallDisplaysWithoutDuplicates(
  primary: ToolCallDisplay[],
  secondary: ToolCallDisplay[],
): ToolCallDisplay[] {
  if (primary.length === 0) return [...secondary];
  if (secondary.length === 0) return [...primary];

  const result = [...primary];
  const primaryState = collectToolCallDisplayMatchState(primary);
  const remainingState = cloneToolCallMatchState(primaryState);

  for (const toolCall of secondary) {
    if (consumeDisplayMatch(toolCall, remainingState)) continue;
    result.push(toolCall);
  }

  return result;
}

export function resolveToolCallInfosForRender(
  source: ToolCallInfoRenderSource,
): ToolCallInfo[] | undefined {
  if (Object.prototype.hasOwnProperty.call(source, "displayToolCalls")) {
    return source.displayToolCalls;
  }
  return source.messageToolCalls;
}

export function buildMessageToolCalls(
  message: Pick<ChatMessage, "toolCalls">,
  toolOutputMap: Record<string, string>,
): ToolCallDisplay[] {
  return (message.toolCalls ?? []).map((toolCall) => buildMessageToolCall(toolCall, toolOutputMap));
}

export function buildMessageToolCall(
  toolCall: ToolCallInfo,
  toolOutputMap: Record<string, string>,
): ToolCallDisplay {
  const output =
    toolCall.recordedOutput
    ?? toolCall.serverToolOutput
    ?? toolOutputMap[toolCall.id];

  return {
    id: toolCall.id,
    name: toolCall.name,
    arguments: toolCall.arguments,
    order: toolCall.order,
    status: inferToolCallStatus(toolCall, output),
    output,
    nestedToolCalls: toolCall.nestedToolCalls?.map((nestedToolCall) =>
      buildMessageToolCall(nestedToolCall, toolOutputMap),
    ),
  };
}

function inferToolCallStatus(
  toolCall: ToolCallInfo,
  output: string | undefined,
): ToolCallDisplay["status"] {
  if (toolCall.outcome) {
    return toolCall.outcome;
  }
  if (output === INTERRUPTED_TOOL_RESULT) {
    return "interrupted";
  }
  return "done";
}

export function mergeSequentialAssistantToolCalls<T extends AssistantToolMergeCandidate>(
  items: T[],
): Array<AssistantToolMergeResult<T>> {
  const merged: Array<AssistantToolMergeResult<T>> = [];
  let pendingToolOnlyItem: T | null = null;
  let pendingToolCalls: ToolCallInfo[] = [];

  const clearPendingToolOnlyItem = () => {
    pendingToolOnlyItem = null;
    pendingToolCalls = [];
  };

  const flushPendingToolOnlyItem = () => {
    if (!pendingToolOnlyItem) return;
    const displayToolCalls = pendingToolCalls.length > 0 ? [...pendingToolCalls] : undefined;
    merged.push({
      ...pendingToolOnlyItem,
      displayToolCalls,
      displayToolCallsBeforeContent: displayToolCalls,
    });
    pendingToolOnlyItem = null;
    pendingToolCalls = [];
  };

  for (const item of items) {
    if (item.renderParts && item.renderParts.length > 0) {
      const hasToolCallsProperty = Object.prototype.hasOwnProperty.call(item, "toolCalls");
      const displayToolCalls = hasToolCallsProperty ? [...(item.toolCalls ?? [])] : undefined;
      flushPendingToolOnlyItem();
      merged.push({
        ...item,
        ...(hasToolCallsProperty ? { displayToolCalls } : {}),
      });
      continue;
    }

    const currentToolCalls = item.toolCalls ?? [];
    const hasResponseText = !item.isKnowledgeProposal && item.content.trim().length > 0;
    const hasThinkingContent = !item.isKnowledgeProposal && !!item.thinkingContent?.trim();
    const isToolOnlyRound =
      !item.isKnowledgeProposal
      && !hasResponseText
      && !hasThinkingContent
      && (item.attachedKnowledgeProposalCount ?? 0) === 0
      && currentToolCalls.length > 0;
    const canAbsorbPendingRounds = !item.isKnowledgeProposal && hasResponseText;

    if (isToolOnlyRound) {
      pendingToolOnlyItem ??= item;
      pendingToolCalls.push(...currentToolCalls);
      continue;
    }

    if (pendingToolCalls.length > 0 && canAbsorbPendingRounds) {
      const beforeContentToolCalls = [...pendingToolCalls];
      const afterContentToolCalls = currentToolCalls.length > 0 ? [...currentToolCalls] : undefined;
      merged.push({
        ...item,
        displayToolCalls: [...beforeContentToolCalls, ...currentToolCalls],
        displayToolCallsBeforeContent: beforeContentToolCalls,
        displayToolCallsAfterContent: afterContentToolCalls,
      });
      clearPendingToolOnlyItem();
      continue;
    }

    flushPendingToolOnlyItem();

    merged.push({
      ...item,
      displayToolCalls: currentToolCalls.length > 0 ? [...currentToolCalls] : undefined,
      displayToolCallsBeforeContent:
        !hasResponseText && currentToolCalls.length > 0 ? [...currentToolCalls] : undefined,
      displayToolCallsAfterContent:
        hasResponseText && currentToolCalls.length > 0 ? [...currentToolCalls] : undefined,
    });
  }

  flushPendingToolOnlyItem();

  return merged;
}

export function summarizeToolCallBatch(
  toolCalls: ToolCallDisplay[],
  compactEnabled: boolean,
): ToolCallBatchState {
  const total = toolCalls.length;
  let doneCount = 0;
  let runningCount = 0;
  let errorCount = 0;
  let interruptedCount = 0;

  for (const toolCall of toolCalls) {
    switch (toolCall.status) {
      case "done":
        doneCount += 1;
        break;
      case "running":
        runningCount += 1;
        break;
      case "error":
        errorCount += 1;
        break;
      case "interrupted":
        interruptedCount += 1;
        break;
    }
  }

  return {
    total,
    doneCount,
    runningCount,
    errorCount,
    interruptedCount,
    canCollapse:
      compactEnabled
      && total >= 2
      && runningCount === 0,
  };
}

````

### src/composables/useStreamReducer.ts

行数：812

````ts
import { hydrateChatMessageIntent, parseUserIntentMeta } from "./chatInputIntents";
import { sortedAssistantRenderParts } from "./assistantRenderParts";
import type { StreamEvent, ChatMessage, TokenUsage, TodoItem, ToolCallDisplay, ToolCallInfo, PendingQuestion, PendingToolConfirm, ImageAttachment, AssetRefAttachment, ToolCallProgress, AssistantRenderPart } from "../types";

export interface StreamState {
  messages: ChatMessage[];
  streamingText: string;
  rawStreamText: string;
  streamingThinking: string;
  streamSequence: number;
  streamingTextOrder: number;
  thinkingOrder: number;
  liveRenderParts: AssistantRenderPart[];
  isStreaming: boolean;
  isCompacting: boolean;
  isThinking: boolean;
  thinkingStartTime: number;
  thinkingDuration: number;
  activeToolCalls: ToolCallDisplay[];
  tokenUsage: TokenUsage;
  todos: TodoItem[];
  showTodoPanel: boolean;
  pendingQuestion: PendingQuestion | null;
  pendingToolConfirms: PendingToolConfirm[];
  undoableMessageIds: Set<string>;
}

export type StreamMutation =
  | { type: "appendRawText"; text: string }
  | { type: "appendThinking"; text: string }
  | { type: "setStreamSequence"; value: number }
  | { type: "setStreamingTextOrder"; order: number }
  | { type: "setThinkingOrder"; order: number }
  | { type: "upsertLiveRenderPart"; part: AssistantRenderPart }
  | { type: "appendLiveRenderPartContent"; partId: string; text: string }
  | { type: "deactivateLiveThinkingParts"; duration?: number }
  | { type: "updateLiveToolPart"; toolCallId: string; updates: Partial<ToolCallInfo> }
  | { type: "clearLiveRenderParts" }
  | { type: "setThinking"; value: boolean; startTime?: number }
  | { type: "updateThinkingDuration"; duration: number }
  | { type: "addToolCall"; toolCall: ToolCallDisplay }
  | { type: "updateToolCall"; id: string; updates: Partial<ToolCallDisplay> }
  | { type: "addNestedToolCall"; parentId: string; toolCall: ToolCallDisplay }
  | { type: "updateNestedToolCall"; parentId: string; childId: string; updates: Partial<ToolCallDisplay> }
  | { type: "appendToolDelta"; id: string; delta: string }
  | { type: "updateToolProgress"; id: string; progress: ToolCallProgress | null }
  | { type: "pushMessage"; message: ChatMessage }
  | { type: "upsertMessage"; message: ChatMessage }
  | { type: "upsertUserMessage"; message: ChatMessage }
  | { type: "replaceMessages"; messages: ChatMessage[] }
  | { type: "pushToolResults"; toolCallIds?: string[] }
  | { type: "resetRound" }
  | { type: "resetRoundKeepToolCalls" }
  | { type: "clearPendingInputs" }
  | { type: "clearPendingInput"; questionId: string }
  | { type: "updateUsage"; usage: TokenUsage }
  | { type: "setQuestion"; question: PendingQuestion | null }
  | { type: "enqueueToolConfirm"; confirm: PendingToolConfirm }
  | { type: "addUndoable"; messageId: string }
  | { type: "setTodos"; runId: string; todos: TodoItem[] }
  | { type: "setStreaming"; value: boolean }
  | { type: "setCompacting"; value: boolean }
  | { type: "canvasAutoOpen"; toolCallId: string; spec: unknown };

export function buildToolResultMessages(
  activeToolCalls: ToolCallDisplay[],
  createdAt = Date.now() / 1000,
): ChatMessage[] {
  return activeToolCalls
    .filter((toolCall) => toolCall.output !== undefined)
    .map((toolCall): ChatMessage => ({
      id: `tool_result_${toolCall.id}`,
      role: "tool",
      content: toolCall.output ?? "",
      createdAt,
      toolCallId: toolCall.id,
    }));
}

function collectToolCallInfoIds(toolCalls: ToolCallInfo[] | undefined): string[] {
  const ids: string[] = [];
  const visit = (items: ToolCallInfo[] | undefined) => {
    for (const item of items ?? []) {
      ids.push(item.id);
      visit(item.nestedToolCalls);
    }
  };
  visit(toolCalls);
  return ids;
}

function pendingUserMessageId(id: string): boolean {
  return id.startsWith("user_pending_") || id.startsWith("embedded_user_");
}

function imageFingerprint(images: ImageAttachment[] | undefined): string {
  return (images ?? [])
    .map((image) => `${image.mimeType}\u{0}${image.data}`)
    .join("\u{1}");
}

function assetRefFingerprint(assetRefs: AssetRefAttachment[] | undefined): string {
  return (assetRefs ?? [])
    .map((assetRef) => `${assetRef.kind}\u{0}${assetRef.path}`)
    .join("\u{1}");
}

function isMatchingPendingUserMessage(candidate: ChatMessage, message: ChatMessage): boolean {
  if (candidate.role !== "user" || !pendingUserMessageId(candidate.id)) return false;
  if (imageFingerprint(candidate.images) !== imageFingerprint(message.images)) return false;
  if (assetRefFingerprint(candidate.assetRefs) !== assetRefFingerprint(message.assetRefs)) return false;

  const candidateClientMessageId = candidate.intentMeta?.clientMessageId
    ?? parseUserIntentMeta(candidate.thinkingSignature)?.clientMessageId;
  const incomingClientMessageId = message.intentMeta?.clientMessageId
    ?? parseUserIntentMeta(message.thinkingSignature)?.clientMessageId;
  if (candidateClientMessageId || incomingClientMessageId) {
    return !!candidateClientMessageId && candidateClientMessageId === incomingClientMessageId;
  }

  if (candidate.content === message.content) return true;
  const candidateContent = candidate.content.trim();
  const incomingContent = message.content.trim();
  return !!candidateContent && !!incomingContent && incomingContent.includes(candidateContent);
}

export function mergeUserMessage(messages: ChatMessage[], incoming: ChatMessage): ChatMessage[] {
  const message = hydrateChatMessageIntent(incoming);
  const existingIndex = messages.findIndex((item) => item.id === message.id);
  if (existingIndex >= 0) {
    const next = [...messages];
    next.splice(existingIndex, 1, message);
    return next;
  }

  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (!isMatchingPendingUserMessage(messages[index]!, message)) continue;
    const next = [...messages];
    next.splice(index, 1, message);
    return next;
  }

  return [...messages, message];
}

function cloneToolCallInfo(toolCall: ToolCallInfo): ToolCallInfo {
  return {
    ...toolCall,
    nestedToolCalls: toolCall.nestedToolCalls?.map(cloneToolCallInfo),
  };
}

function liveOrderFromEvent(
  event: { runId: string; order?: number; renderSeq?: number; partId?: string },
  fallbackSeq: number,
  fallbackPartId: string,
) {
  const seq =
    typeof event.renderSeq === "number" && event.renderSeq > 0
      ? event.renderSeq
      : fallbackSeq;

  if (import.meta.env.DEV && (typeof event.renderSeq !== "number" || event.renderSeq <= 0)) {
    console.error("[render-parts] stream event missing renderSeq", event);
  }

  return {
    id: event.partId?.trim() || fallbackPartId,
    order: { runId: event.runId, seq },
  };
}

function existingLivePart<T extends AssistantRenderPart["kind"]>(
  state: StreamState,
  kind: T,
  id: string,
): Extract<AssistantRenderPart, { kind: T }> | undefined {
  return state.liveRenderParts.find(
    (part): part is Extract<AssistantRenderPart, { kind: T }> =>
      part.kind === kind && part.id === id,
  );
}

function currentThinkingDuration(state: StreamState) {
  return state.isThinking && state.thinkingStartTime > 0
    ? Math.round((Date.now() - state.thinkingStartTime) / 1000)
    : undefined;
}

function finalizeLiveRenderParts(
  state: StreamState,
  options: {
    runId: string;
    messageId: string;
    fullText: string;
    toolCalls?: ToolCallInfo[];
    renderParts?: AssistantRenderPart[] | null;
    contentOrder?: number;
    thinkingOrder?: number;
    thinkingContent?: string | null;
    thinkingDuration?: number | null;
  },
): AssistantRenderPart[] {
  if (options.renderParts?.length) {
    return sortedAssistantRenderParts(options.renderParts);
  }

  const toolCallsById = new Map((options.toolCalls ?? []).map((toolCall) => [toolCall.id, toolCall]));
  const parts = state.liveRenderParts.map((part): AssistantRenderPart => {
    if (part.kind === "thinking") {
      return {
        ...part,
        active: false,
        content: options.thinkingContent ?? part.content,
        duration: options.thinkingDuration ?? state.thinkingDuration,
      };
    }
    if (part.kind === "text") {
      return { ...part, content: options.fullText || part.content };
    }
    if (part.kind === "toolCall") {
      return {
        ...part,
        toolCall: cloneToolCallInfo(toolCallsById.get(part.id) ?? part.toolCall),
      };
    }
    return part;
  });

  const hasTextPart = parts.some((part) => part.kind === "text");
  if (options.fullText && !hasTextPart) {
    const seq = options.contentOrder && options.contentOrder > 0 ? options.contentOrder : state.streamSequence + 1;
    parts.push({
      kind: "text",
      id: `${options.messageId}:text`,
      order: { runId: options.runId, seq },
      content: options.fullText,
    });
  }

  const thinkingContent = options.thinkingContent ?? state.streamingThinking;
  const hasThinkingPart = parts.some((part) => part.kind === "thinking");
  if (thinkingContent && !hasThinkingPart) {
    const seq = options.thinkingOrder && options.thinkingOrder > 0 ? options.thinkingOrder : state.streamSequence + 1;
    parts.push({
      kind: "thinking",
      id: `${options.messageId}:thinking`,
      order: { runId: options.runId, seq },
      content: thinkingContent,
      active: false,
      duration: options.thinkingDuration ?? state.thinkingDuration,
    });
  }

  const existingToolPartIds = new Set(
    parts.filter((part) => part.kind === "toolCall").map((part) => part.id),
  );
  for (const toolCall of options.toolCalls ?? []) {
    if (existingToolPartIds.has(toolCall.id)) continue;
    const seq = toolCall.order && toolCall.order > 0 ? toolCall.order : state.streamSequence + 1;
    parts.push({
      kind: "toolCall",
      id: toolCall.id,
      order: { runId: options.runId, seq },
      toolCall: cloneToolCallInfo(toolCall),
    });
  }

  return sortedAssistantRenderParts(parts);
}

export function reduceStreamEvent(state: StreamState, event: StreamEvent): StreamMutation[] {
  const mutations: StreamMutation[] = [];

  let streamSequenceCursor = state.streamSequence;

  const nextStreamOrder = () => streamSequenceCursor + 1;

  const markStreamSequence = (order: number) => {
    if (order > streamSequenceCursor) {
      streamSequenceCursor = order;
      mutations.push({ type: "setStreamSequence", value: order });
    }
  };

  const resolveOrder = (explicitOrder?: number) => (
    typeof explicitOrder === "number" && explicitOrder > streamSequenceCursor
      ? explicitOrder
      : nextStreamOrder()
  );

  const resolveMessageRenderOrders = (options: {
    hasContent: boolean;
    contentCurrentOrder?: number;
    contentExplicitOrder?: number;
    hasThinking: boolean;
    thinkingCurrentOrder?: number;
    thinkingExplicitOrder?: number;
  }) => {
    let contentOrder = options.hasContent && options.contentCurrentOrder && options.contentCurrentOrder > 0
      ? options.contentCurrentOrder
      : undefined;
    let thinkingOrder = options.hasThinking && options.thinkingCurrentOrder && options.thinkingCurrentOrder > 0
      ? options.thinkingCurrentOrder
      : undefined;
    const pendingOrders: Array<{
      target: "thinking" | "content";
      explicitOrder?: number;
      fallbackRank: number;
    }> = [];

    if (options.hasThinking && !thinkingOrder) {
      pendingOrders.push({
        target: "thinking",
        explicitOrder: options.thinkingExplicitOrder,
        fallbackRank: 0,
      });
    }
    if (options.hasContent && !contentOrder) {
      pendingOrders.push({
        target: "content",
        explicitOrder: options.contentExplicitOrder,
        fallbackRank: 1,
      });
    }

    pendingOrders.sort((left, right) => {
      const leftOrder = typeof left.explicitOrder === "number" && left.explicitOrder > 0
        ? left.explicitOrder
        : Number.POSITIVE_INFINITY;
      const rightOrder = typeof right.explicitOrder === "number" && right.explicitOrder > 0
        ? right.explicitOrder
        : Number.POSITIVE_INFINITY;
      return leftOrder - rightOrder || left.fallbackRank - right.fallbackRank;
    });

    for (const pending of pendingOrders) {
      const order = resolveOrder(pending.explicitOrder);
      markStreamSequence(order);
      if (pending.target === "thinking") {
        thinkingOrder = order;
      } else {
        contentOrder = order;
      }
    }

    return { contentOrder, thinkingOrder };
  };

  const markTextOrder = (explicitOrder?: number) => {
    if (state.streamingTextOrder > 0 || state.rawStreamText.length > 0) return;
    const order = resolveOrder(explicitOrder);
    mutations.push({ type: "setStreamingTextOrder", order });
    markStreamSequence(order);
  };

  const markThinkingOrder = (explicitOrder?: number) => {
    if (state.thinkingOrder > 0 || state.streamingThinking.length > 0) return;
    const order = resolveOrder(explicitOrder);
    mutations.push({ type: "setThinkingOrder", order });
    markStreamSequence(order);
  };

  const markToolOrder = (existing?: ToolCallDisplay, explicitOrder?: number) => {
    if (existing?.order && existing.order > 0) return existing.order;
    const order = resolveOrder(explicitOrder);
    markStreamSequence(order);
    return order;
  };

  const finishThinkingBeforeTools = () => {
    const duration = currentThinkingDuration(state);
    if (duration !== undefined) {
      mutations.push({ type: "updateThinkingDuration", duration });
      mutations.push({ type: "deactivateLiveThinkingParts", duration });
    } else {
      mutations.push({ type: "deactivateLiveThinkingParts" });
    }
    if (state.isThinking) {
      mutations.push({ type: "setThinking", value: false });
    }
  };

  // Note: auto-reactivation of streaming removed — streaming is now controlled
  // exclusively by explicit sendChat/cancelChat actions. Late events from a
  // cancelled run are filtered by runId in the chat store.

  switch (event.type) {
    case "userMessage":
      mutations.push({ type: "upsertUserMessage", message: event.message });
      break;

    case "textDelta":
      markTextOrder(event.order);
      {
        const order = liveOrderFromEvent(
          event,
          state.streamingTextOrder || nextStreamOrder(),
          `${event.runId}:text`,
        );
        mutations.push({ type: "deactivateLiveThinkingParts", duration: currentThinkingDuration(state) });
        mutations.push({
          type: "upsertLiveRenderPart",
          part: {
            kind: "text",
            id: order.id,
            order: order.order,
            content: existingLivePart(state, "text", order.id)?.content ?? "",
          },
        });
        mutations.push({ type: "appendLiveRenderPartContent", partId: order.id, text: event.text });
      }
      mutations.push({ type: "appendRawText", text: event.text });
      if (state.isThinking && state.thinkingStartTime > 0) {
        mutations.push({ type: "updateThinkingDuration", duration: Math.round((Date.now() - state.thinkingStartTime) / 1000) });
      }
      mutations.push({ type: "setThinking", value: false });
      break;

    case "thinkingDelta":
      markThinkingOrder(event.order);
      {
        const order = liveOrderFromEvent(
          event,
          state.thinkingOrder || nextStreamOrder(),
          `${event.runId}:thinking`,
        );
        mutations.push({
          type: "upsertLiveRenderPart",
          part: {
            kind: "thinking",
            id: order.id,
            order: order.order,
            content: existingLivePart(state, "thinking", order.id)?.content ?? "",
            active: true,
            duration: state.thinkingDuration > 0 ? state.thinkingDuration : undefined,
          },
        });
        mutations.push({ type: "appendLiveRenderPartContent", partId: order.id, text: event.text });
      }
      mutations.push({ type: "appendThinking", text: event.text });
      if (!state.isThinking) {
        mutations.push({ type: "setThinking", value: true, startTime: Date.now() });
      }
      break;

    case "toolCallStart": {
      finishThinkingBeforeTools();
      const existing = state.activeToolCalls.find((t) => t.id === event.toolCallId);
      const legacyOrder = markToolOrder(existing, event.order);
      const liveOrder = liveOrderFromEvent(event, legacyOrder, event.toolCallId);
      const currentPart = existingLivePart(state, "toolCall", liveOrder.id);
      mutations.push({
        type: "upsertLiveRenderPart",
        part: {
          kind: "toolCall",
          id: liveOrder.id,
          order: liveOrder.order,
          toolCall: {
            ...(currentPart?.toolCall ?? {
              id: event.toolCallId,
              name: event.toolName,
              arguments: "",
            }),
            id: event.toolCallId,
            name: event.toolName,
            arguments: event.arguments || currentPart?.toolCall.arguments || "",
            order: liveOrder.order.seq,
          },
        },
      });
      if (existing) {
        const updates: Partial<ToolCallDisplay> = {};
        if (event.arguments) {
          updates.arguments = event.arguments;
        }
        if (!existing.order || existing.order <= 0 || existing.order !== liveOrder.order.seq) {
          updates.order = liveOrder.order.seq;
        }
        if (Object.keys(updates).length > 0) {
          mutations.push({ type: "updateToolCall", id: event.toolCallId, updates });
        }
      } else {
        mutations.push({
          type: "addToolCall",
          toolCall: { id: event.toolCallId, name: event.toolName, arguments: event.arguments, status: "running", order: liveOrder.order.seq },
        });
      }
      break;
    }

    case "toolCallDone": {
      mutations.push({
        type: "updateLiveToolPart",
        toolCallId: event.toolCallId,
        updates: { outcome: event.outcome, recordedOutput: event.output },
      });
      mutations.push({
        type: "updateToolCall",
        id: event.toolCallId,
        updates: { status: event.outcome, output: event.output, progress: null },
      });
      // Parse todowrite output
      if (event.toolName === "todowrite" && event.outcome === "done") {
        const jsonStart = event.output.indexOf("[");
        if (jsonStart >= 0) {
          try {
            const parsed = JSON.parse(event.output.slice(jsonStart)) as TodoItem[];
            mutations.push({ type: "setTodos", runId: event.runId, todos: parsed });
          } catch { /* ignore */ }
        }
      }
      // Canvas auto-open
      if (event.toolName === "canvas" && event.outcome === "done") {
        const canvasTc = state.activeToolCalls.find((t) => t.id === event.toolCallId);
        if (canvasTc) {
          try {
            const parsed = JSON.parse(canvasTc.arguments);
            if (parsed.spec) {
              mutations.push({ type: "canvasAutoOpen", toolCallId: event.toolCallId, spec: parsed.spec });
            }
          } catch { /* ignore */ }
        }
      }
      break;
    }

    case "toolCallDelta":
      mutations.push({ type: "appendToolDelta", id: event.toolCallId, delta: event.delta });
      break;

    case "toolCallProgress":
      mutations.push({
        type: "updateToolProgress",
        id: event.toolCallId,
        progress: {
          title: event.title,
          info: event.info,
          progress: event.progress,
          state: event.state,
        },
      });
      break;

    case "subagentToolCallStart": {
      finishThinkingBeforeTools();
      const parentTc = state.activeToolCalls.find((t) => t.id === event.parentToolCallId);
      if (parentTc) {
        const existingNested = parentTc.nestedToolCalls?.find((t) => t.id === event.toolCallId);
        if (existingNested) {
          if (event.arguments) {
            mutations.push({ type: "updateNestedToolCall", parentId: event.parentToolCallId, childId: event.toolCallId, updates: { arguments: event.arguments } });
          }
        } else {
          const order = markToolOrder(undefined, event.order);
          mutations.push({
            type: "addNestedToolCall",
            parentId: event.parentToolCallId,
            toolCall: { id: event.toolCallId, name: event.toolName, arguments: event.arguments, status: "running", order },
          });
        }
      }
      break;
    }

    case "subagentToolCallDone": {
      mutations.push({
        type: "updateNestedToolCall",
        parentId: event.parentToolCallId,
        childId: event.toolCallId,
        updates: { status: event.outcome, output: event.output },
      });
      break;
    }

    case "toolCallRoundDone": {
      if (state.isThinking && state.thinkingStartTime > 0) {
        mutations.push({ type: "updateThinkingDuration", duration: Math.round((Date.now() - state.thinkingStartTime) / 1000) });
      }
      const messageOrders = resolveMessageRenderOrders({
        hasContent: !!event.fullText,
        contentCurrentOrder: state.streamingTextOrder,
        contentExplicitOrder: event.contentOrder,
        hasThinking: !!state.streamingThinking,
        thinkingCurrentOrder: state.thinkingOrder,
        thinkingExplicitOrder: event.thinkingOrder,
      });
      const renderParts = finalizeLiveRenderParts(state, {
        runId: event.runId,
        messageId: event.messageId,
        fullText: event.fullText,
        toolCalls: event.toolCalls,
        renderParts: event.renderParts,
        contentOrder: messageOrders.contentOrder,
        thinkingOrder: messageOrders.thinkingOrder,
      });
      mutations.push({
        type: "pushMessage",
        message: {
          id: event.messageId,
          role: "assistant",
          content: event.fullText,
          createdAt: Date.now() / 1000,
          toolCalls: event.toolCalls.length > 0 ? event.toolCalls : undefined,
          thinkingContent: state.streamingThinking || undefined,
          thinkingDuration: state.thinkingDuration > 0 ? state.thinkingDuration : undefined,
          contentOrder: messageOrders.contentOrder,
          thinkingOrder: messageOrders.thinkingOrder,
          renderParts,
        },
      });
      mutations.push({ type: "pushToolResults", toolCallIds: collectToolCallInfoIds(event.toolCalls) });
      mutations.push({ type: "clearLiveRenderParts" });
      mutations.push({ type: "resetRound" });
      break;
    }

    case "knowledgeProposal":
      mutations.push({ type: "upsertMessage", message: event.message });
      break;

    case "usageUpdate":
      mutations.push({
        type: "updateUsage",
        usage: {
          totalInputTokens: event.totalInputTokens,
          totalOutputTokens: event.totalOutputTokens,
          totalCacheReadTokens: event.totalCacheReadTokens,
          totalCacheWriteTokens: event.totalCacheWriteTokens,
          totalCostUsd: event.totalCostUsd,
          pricedRounds: event.pricedRounds,
          contextTokens: event.contextTokens > 0 ? event.contextTokens : state.tokenUsage.contextTokens,
          contextLimit: event.contextLimit > 0 ? event.contextLimit : state.tokenUsage.contextLimit,
        },
      });
      break;

    case "compactStart":
      mutations.push({ type: "setCompacting", value: true });
      mutations.push({
        type: "updateUsage",
        usage: {
          ...state.tokenUsage,
          contextTokens: event.contextTokens > 0 ? event.contextTokens : state.tokenUsage.contextTokens,
          contextLimit: event.contextLimit > 0 ? event.contextLimit : state.tokenUsage.contextLimit,
        },
      });
      break;

    case "compactDone":
      mutations.push({ type: "replaceMessages", messages: event.messages });
      if ((event.contextTokens ?? 0) > 0 && (event.contextLimit ?? 0) > 0) {
        mutations.push({
          type: "updateUsage",
          usage: {
            ...state.tokenUsage,
            contextTokens: event.contextTokens ?? state.tokenUsage.contextTokens,
            contextLimit: event.contextLimit ?? state.tokenUsage.contextLimit,
          },
        });
      }
      mutations.push({ type: "setCompacting", value: false });
      break;

    case "askUser":
      mutations.push({
        type: "setQuestion",
        question: {
          questionId: event.questionId,
          toolCallId: event.toolCallId,
          question: event.question,
          options: event.options,
        },
      });
      break;

    case "toolConfirm":
      mutations.push({
        type: "enqueueToolConfirm",
        confirm: {
          questionId: event.questionId,
          toolCallId: event.toolCallId,
          display: event.display,
        },
      });
      break;

    case "inputAnswered":
      mutations.push({ type: "clearPendingInput", questionId: event.questionId });
      break;

    case "undoAvailable":
      mutations.push({ type: "addUndoable", messageId: event.assistantMessageId });
      break;

    case "done": {
      if (state.isThinking && state.thinkingStartTime > 0) {
        mutations.push({ type: "updateThinkingDuration", duration: Math.round((Date.now() - state.thinkingStartTime) / 1000) });
      }
      if (event.fullText || event.renderParts?.length) {
        const existingMessage = state.messages.find((message) => message.id === event.messageId);
        const messageOrders = resolveMessageRenderOrders({
          hasContent: !!event.fullText,
          contentCurrentOrder: existingMessage?.contentOrder ?? state.streamingTextOrder,
          contentExplicitOrder: event.contentOrder,
          hasThinking: !!(existingMessage?.thinkingContent ?? state.streamingThinking),
          thinkingCurrentOrder: existingMessage?.thinkingOrder ?? state.thinkingOrder,
          thinkingExplicitOrder: event.thinkingOrder,
        });
        const renderParts = finalizeLiveRenderParts(state, {
          runId: event.runId,
          messageId: event.messageId,
          fullText: event.fullText,
          toolCalls: existingMessage?.toolCalls,
          renderParts: event.renderParts ?? existingMessage?.renderParts,
          contentOrder: messageOrders.contentOrder,
          thinkingOrder: messageOrders.thinkingOrder,
          thinkingContent: existingMessage?.thinkingContent ?? state.streamingThinking,
          thinkingDuration: existingMessage?.thinkingDuration ?? state.thinkingDuration,
        });
        mutations.push({
          type: "upsertMessage",
          message: {
            ...existingMessage,
            id: event.messageId,
            role: "assistant",
            content: event.fullText,
            createdAt: existingMessage?.createdAt ?? Date.now() / 1000,
            thinkingContent: (existingMessage?.thinkingContent ?? state.streamingThinking) || undefined,
            thinkingDuration: existingMessage?.thinkingDuration ?? (state.thinkingDuration > 0 ? state.thinkingDuration : undefined),
            contentOrder: messageOrders.contentOrder,
            thinkingOrder: messageOrders.thinkingOrder,
            renderParts,
          },
        });
      }
      mutations.push({ type: "clearLiveRenderParts" });
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
      mutations.push({ type: "setCompacting", value: false });
      break;
    }

    case "cancelled": {
      const hasInterruptedMessage =
        !!event.messageId
        && (
          event.fullText !== undefined
          || event.thinkingContent !== undefined
          || state.rawStreamText.length > 0
          || state.streamingThinking.length > 0
        );
      if (hasInterruptedMessage) {
        const existingMessage = state.messages.find((message) => message.id === event.messageId);
        const content = event.fullText ?? state.rawStreamText;
        const thinkingContent = (event.thinkingContent ?? state.streamingThinking) || undefined;
        const thinkingDuration =
          event.thinkingDuration ?? (state.thinkingDuration > 0 ? state.thinkingDuration : undefined);
        const messageOrders = resolveMessageRenderOrders({
          hasContent: !!content,
          contentCurrentOrder: existingMessage?.contentOrder ?? state.streamingTextOrder,
          hasThinking: !!thinkingContent,
          thinkingCurrentOrder: existingMessage?.thinkingOrder ?? state.thinkingOrder,
        });
        const renderParts = finalizeLiveRenderParts(state, {
          runId: event.runId,
          messageId: event.messageId!,
          fullText: content,
          toolCalls: existingMessage?.toolCalls,
          renderParts: event.renderParts ?? existingMessage?.renderParts,
          contentOrder: messageOrders.contentOrder,
          thinkingOrder: messageOrders.thinkingOrder,
          thinkingContent,
          thinkingDuration,
        });
        mutations.push({
          type: "upsertMessage",
          message: {
            ...existingMessage,
            id: event.messageId!,
            role: "assistant",
            content,
            createdAt: existingMessage?.createdAt ?? Date.now() / 1000,
            thinkingContent,
            thinkingDuration,
            contentOrder: messageOrders.contentOrder,
            thinkingOrder: messageOrders.thinkingOrder,
            renderParts,
          },
        });
      }
      mutations.push({ type: "clearLiveRenderParts" });
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
      mutations.push({ type: "setCompacting", value: false });
      break;
    }

    case "error":
      mutations.push({ type: "clearLiveRenderParts" });
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
      mutations.push({ type: "setCompacting", value: false });
      break;
  }

  return mutations;
}

````

### src/composables/assistantRenderParts.ts

行数：126

````ts
import type { AssistantRenderPart, ChatMessage, RenderOrderKey, ToolCallInfo } from "../types";

const LEGACY_RUN_ID = "legacy";

export function compareRenderOrder(left: RenderOrderKey, right: RenderOrderKey) {
  if (left.runId === right.runId) {
    return left.seq - right.seq;
  }
  if (left.runId === LEGACY_RUN_ID) return 1;
  if (right.runId === LEGACY_RUN_ID) return -1;
  return left.runId.localeCompare(right.runId) || left.seq - right.seq;
}

export function compareAssistantRenderParts(left: AssistantRenderPart, right: AssistantRenderPart) {
  return compareRenderOrder(left.order, right.order);
}

export function sortedAssistantRenderParts(parts: readonly AssistantRenderPart[]) {
  return [...parts].sort(compareAssistantRenderParts);
}

function legacyOrder(seq: number): RenderOrderKey {
  return { runId: LEGACY_RUN_ID, seq };
}

function toolCallOrder(toolCall: ToolCallInfo): number {
  return typeof toolCall.order === "number" && toolCall.order > 0 ? toolCall.order : 0;
}

function hasAnyLegacyOrder(message: Pick<ChatMessage, "contentOrder" | "thinkingOrder" | "toolCalls">) {
  if (message.contentOrder && message.contentOrder > 0) return true;
  if (message.thinkingOrder && message.thinkingOrder > 0) return true;
  return (message.toolCalls ?? []).some((toolCall) => toolCallOrder(toolCall) > 0);
}

export interface LegacyRenderPartOptions {
  toolCalls?: ToolCallInfo[];
  beforeContentToolCalls?: ToolCallInfo[];
  afterContentToolCalls?: ToolCallInfo[];
  knowledgeProposals?: ChatMessage[];
}

export function synthesizeLegacyRenderParts(
  message: ChatMessage,
  options: LegacyRenderPartOptions = {},
): AssistantRenderPart[] {
  if (message.renderParts?.length) {
    return sortedAssistantRenderParts(message.renderParts);
  }

  const ordered = hasAnyLegacyOrder(message);
  const parts: AssistantRenderPart[] = [];
  const contentSeq = ordered && message.contentOrder && message.contentOrder > 0 ? message.contentOrder : 20;
  const thinkingSeq = ordered && message.thinkingOrder && message.thinkingOrder > 0 ? message.thinkingOrder : 10;

  if (message.thinkingContent?.trim()) {
    parts.push({
      kind: "thinking",
      id: `${message.id}:legacy-thinking`,
      order: legacyOrder(thinkingSeq),
      content: message.thinkingContent,
      duration: message.thinkingDuration,
      signature: message.thinkingSignature,
    });
  }

  const pushToolParts = (toolCalls: ToolCallInfo[] | undefined, fallbackSeq: number) => {
    for (const [index, toolCall] of (toolCalls ?? []).entries()) {
      const seq = ordered ? (toolCallOrder(toolCall) || fallbackSeq + index) : fallbackSeq + index;
      parts.push({
        kind: "toolCall",
        id: toolCall.id || `${message.id}:legacy-tool:${index}`,
        order: legacyOrder(seq),
        toolCall,
      });
    }
  };

  if (options.beforeContentToolCalls) {
    pushToolParts(options.beforeContentToolCalls, Math.max(contentSeq - 1, 1));
  }

  if (message.content) {
    parts.push({
      kind: "text",
      id: `${message.id}:legacy-text`,
      order: legacyOrder(contentSeq),
      content: message.content,
    });
  }

  if (options.afterContentToolCalls) {
    pushToolParts(options.afterContentToolCalls, contentSeq + 1);
  } else if (!options.beforeContentToolCalls) {
    pushToolParts(options.toolCalls ?? message.toolCalls, ordered ? 30 : 30);
  }

  for (const [index, proposalMessage] of (options.knowledgeProposals ?? []).entries()) {
    if (!proposalMessage.knowledgeProposal) continue;
    parts.push({
      kind: "knowledgeProposal",
      id: proposalMessage.id,
      order: legacyOrder(contentSeq + 100 + index),
      message: proposalMessage,
    });
  }

  return sortedAssistantRenderParts(parts);
}

export function assertCanonicalRenderParts(parts: readonly AssistantRenderPart[], source: string) {
  if (!import.meta.env.DEV) return;
  const seen = new Set<string>();
  for (const part of parts) {
    if (!part.id || !part.order.runId || !Number.isFinite(part.order.seq) || part.order.seq <= 0) {
      console.error(`[render-parts] invalid ${source} render part`, part);
    }
    const key = `${part.order.runId}:${part.order.seq}`;
    if (seen.has(key)) {
      console.error(`[render-parts] duplicate ${source} renderSeq`, key, parts);
    }
    seen.add(key);
  }
}


````

### src/composables/chatViewStability.ts

行数：257

````ts
import type { ChatMessage, ToolCallDisplay } from "../types";
import { isNearBottom, type ScrollMetrics, type SessionScrollState } from "./chatScrollState";

type ToolCallRuntimeStatus = Pick<ToolCallDisplay, "status" | "nestedToolCalls">;

export function shouldShowAssistantContinuation(
  lastGroupRole: "user" | "assistant" | null,
  hasTransientAssistantMessage: boolean,
): boolean {
  return hasTransientAssistantMessage && lastGroupRole === "assistant";
}

export interface PendingContinuationToolItem {
  id: string;
  content: string;
  toolCallCount: number;
}

export interface PendingContinuationRenderSegment {
  type: "toolCalls" | "content" | "other";
  itemIds?: readonly string[];
}

type TrailingToolMessage = Pick<ChatMessage, "id" | "role" | "toolCalls">;

export function findTrailingAssistantToolMessageId(
  messages: TrailingToolMessage[],
): string | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!message || message.role === "tool") continue;
    if (message.role !== "assistant") return null;
    return message.toolCalls && message.toolCalls.length > 0 ? message.id : null;
  }

  return null;
}

export function collectPendingContinuationToolItemIds(params: {
  isStreaming: boolean;
  lastGroupRole: "user" | "assistant" | null;
  hasTransientAssistantMessage: boolean;
  items: PendingContinuationToolItem[];
}): Set<string> {
  const {
    isStreaming,
    lastGroupRole,
    hasTransientAssistantMessage,
    items,
  } = params;

  if (!isStreaming || !shouldShowAssistantContinuation(lastGroupRole, hasTransientAssistantMessage)) {
    return new Set();
  }

  const pendingIds = new Set<string>();
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (!item) continue;
    if (item.content.trim().length > 0) break;
    if (item.toolCallCount > 0) {
      pendingIds.add(item.id);
    }
  }

  return pendingIds;
}

export function collectPendingContinuationToolSegmentItemIds(params: {
  isStreaming: boolean;
  lastGroupRole: "user" | "assistant" | null;
  hasTransientAssistantMessage: boolean;
  segments: PendingContinuationRenderSegment[];
}): Set<string> {
  const {
    isStreaming,
    lastGroupRole,
    hasTransientAssistantMessage,
    segments,
  } = params;

  if (!isStreaming || !shouldShowAssistantContinuation(lastGroupRole, hasTransientAssistantMessage)) {
    return new Set();
  }

  const pendingIds = new Set<string>();
  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index];
    if (!segment) continue;
    if (segment.type === "content") break;
    if (segment.type !== "toolCalls") continue;
    for (const itemId of segment.itemIds ?? []) {
      pendingIds.add(itemId);
    }
  }

  return pendingIds;
}

export function shouldAutoScrollToBottom(params: {
  force?: boolean;
  metrics: ScrollMetrics;
  remembered: SessionScrollState | null | undefined;
}): boolean {
  const { force = false, metrics, remembered } = params;
  if (!force && remembered && remembered.mode !== "bottom" && !isNearBottom(metrics)) {
    return false;
  }
  return true;
}

export function shouldShowWaitingPlaceholder(params: {
  isStreaming: boolean;
  hasStreamingContent: boolean;
  isThinking: boolean;
  hasThinkingContent: boolean;
}): boolean {
  const {
    isStreaming,
    hasStreamingContent,
    isThinking,
    hasThinkingContent,
  } = params;

  return (
    isStreaming
    && !hasStreamingContent
    && !isThinking
    && !hasThinkingContent
  );
}

export function hasRunningToolCall(toolCalls: ToolCallRuntimeStatus[]): boolean {
  return toolCalls.some((toolCall) =>
    toolCall.status === "running"
    || hasRunningToolCall(toolCall.nestedToolCalls ?? []),
  );
}

type FrameRequest = (cb: FrameRequestCallback) => number;
type FrameCancel = (id: number) => void;
type TimeoutRequest = (cb: () => void, delay: number) => number;
type TimeoutCancel = (id: number) => void;

function defaultFrameRequest(cb: FrameRequestCallback): number {
  if (typeof requestAnimationFrame === "function") {
    return requestAnimationFrame(cb);
  }
  if (typeof window !== "undefined") {
    return window.setTimeout(() => cb(Date.now()), 16);
  }
  return globalThis.setTimeout(() => cb(Date.now()), 16) as unknown as number;
}

function defaultFrameCancel(id: number) {
  if (typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(id);
    return;
  }
  clearTimeout(id);
}

function defaultTimeoutRequest(cb: () => void, delay: number): number {
  if (typeof window !== "undefined") {
    return window.setTimeout(cb, delay);
  }
  return globalThis.setTimeout(cb, delay) as unknown as number;
}

function defaultTimeoutCancel(id: number) {
  if (typeof window !== "undefined") {
    window.clearTimeout(id);
    return;
  }
  globalThis.clearTimeout(id as unknown as ReturnType<typeof setTimeout>);
}

export function createCoalescedScrollScheduler(
  run: (force: boolean) => void,
  requestFrame: FrameRequest = defaultFrameRequest,
  cancelFrame: FrameCancel = defaultFrameCancel,
) {
  let frameId = 0;
  let pendingForce = false;

  function flush() {
    if (!frameId) return;
    frameId = 0;
    const force = pendingForce;
    pendingForce = false;
    run(force);
  }

  return {
    schedule(force = false) {
      pendingForce = pendingForce || force;
      if (frameId) return;
      frameId = requestFrame(() => flush());
    },
    cancel() {
      if (!frameId) return;
      cancelFrame(frameId);
      frameId = 0;
      pendingForce = false;
    },
  };
}

export function createSettledScrollScheduler(
  run: () => void,
  settleDelayMs: number,
  requestFrame: FrameRequest = defaultFrameRequest,
  cancelFrame: FrameCancel = defaultFrameCancel,
  requestTimeout: TimeoutRequest = defaultTimeoutRequest,
  cancelTimeout: TimeoutCancel = defaultTimeoutCancel,
) {
  let frameId = 0;
  let timeoutId: number | null = null;
  let scheduleVersion = 0;

  return {
    schedule() {
      scheduleVersion += 1;
      const currentVersion = scheduleVersion;
      run();

      if (frameId) {
        cancelFrame(frameId);
      }
      frameId = requestFrame(() => {
        if (scheduleVersion !== currentVersion) return;
        frameId = 0;
        run();
      });

      if (timeoutId) {
        cancelTimeout(timeoutId);
      }
      timeoutId = requestTimeout(() => {
        if (scheduleVersion !== currentVersion) return;
        timeoutId = null;
        run();
      }, settleDelayMs);
    },
    cancel() {
      scheduleVersion += 1;
      if (frameId) {
        cancelFrame(frameId);
        frameId = 0;
      }
      if (!timeoutId) return;
      cancelTimeout(timeoutId);
      timeoutId = null;
    },
  };
}

````

### src/services/toolCollapseTrace.ts

行数：70

````ts
const traceStartMs =
  typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();

const TOOL_COLLAPSE_HANDOFF_EVENTS = new Set([
  "activeToolCallsCleared",
  "activeToolCallsResumedWithHandoff",
  "animateCollapseOnMount",
  "beginToolCallHandoff",
  "clearToolCallHandoff",
  "collapseArmed",
  "expandedChanged",
  "historyToolSegmentPinnedStateChanged",
  "historyToolSegmentExpansionDecision",
  "onTransientToolCallsCollapseFinished",
  "panelAfterLeave",
  "pendingContinuationToolItemIdsChanged",
  "promotableHistoryToolCallsChanged",
  "promotedHistoryToolCallsRenderGap",
  "promotedHistoryToolCallsVisibilityChanged",
  "clearRetainedCollapsedToolCalls",
  "retainCollapsedToolCallHandoff",
  "retainCollapsedToolCallHandoffSkipped",
  "transientPromotedToolCallsCoverage",
  "transientToolCallsCollapseEnabledChanged",
  "waitingLayoutStateChanged",
]);

function shouldTraceEvent(event: string) {
  if (typeof localStorage === "undefined") return false;

  const mode = localStorage.getItem("locus.toolCollapseTrace");
  if (mode === "all") return true;
  if (mode === "handoff") return TOOL_COLLAPSE_HANDOFF_EVENTS.has(event);
  if (mode === "waiting") return event === "waitingLayoutStateChanged";
  return false;
}

function nowMs() {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

function elapsedMs() {
  return Math.round((nowMs() - traceStartMs) * 10) / 10;
}

export function previewTraceText(text: string, maxLength = 80) {
  const compact = text.replace(/\s+/g, " ").trim();
  if (compact.length <= maxLength) return compact;
  return `${compact.slice(0, maxLength - 1)}…`;
}

export function logToolCollapseTrace(
  scope: string,
  event: string,
  detail?: Record<string, unknown>,
) {
  if (!shouldTraceEvent(event)) return;

  const prefix = `[tool-collapse][+${elapsedMs()}ms][${scope}] ${event}`;
  if (!detail || Object.keys(detail).length === 0) {
    console.info(prefix);
    return;
  }
  console.info(prefix, detail);
}

````

### src/composables/useDisplaySettings.ts

行数：156

````ts
import { reactive } from "vue";

export type FontSlot = "ui" | "prose" | "monoInline" | "monoBlock" | "monoEditor";
export type DiffReviewTarget = "inline" | "window";
export type ChatDiffReviewTarget = DiffReviewTarget;
export type GitDiffReviewTarget = DiffReviewTarget;

export interface DisplaySettings {
  /** Auto-open TODO panel when todos arrive */
  todoAutoOpen: boolean;
  /** Auto-open file changes panel when changes arrive */
  changesAutoOpen: boolean;
  /** Auto-close file changes panel when a new round starts */
  changesAutoClose: boolean;
  /** Default target for reviewing chat file diffs */
  chatDiffReviewTarget: DiffReviewTarget;
  /** Default target for reviewing Git file diffs */
  gitDiffReviewTarget: DiffReviewTarget;
  /** Right-align user messages in the session transcript */
  rightAlignUserMessages: boolean;
  /** Collapse completed tool call batches in chat transcript */
  compactToolCalls: boolean;
  /** Hide completed thinking blocks in chat transcript */
  hideThinkingBlocks: boolean;
  /** Merge Git tree status letters into colored file icons */
  mergeGitTreeStatusIcon: boolean;
  /** Hide Git command suggestions in Git terminal */
  hideGitCommandSuggestions: boolean;
  /** Enable desktop notifications when the app is not focused */
  systemNotificationsEnabled: boolean;
  /** Notify when a chat run completes */
  notifyOnChatDone: boolean;
  /** Notify when a subagent run completes */
  notifyOnSubagentDone: boolean;
  /** Notify when the agent asks the user a question */
  notifyOnAskUser: boolean;
  /** Notify when a chat run fails */
  notifyOnChatError: boolean;
  /** Notify when tool approval is required */
  notifyOnToolConfirm: boolean;
  /** Per-slot font-family overrides (empty string = use default) */
  fonts: Record<FontSlot, string>;
}

const STORAGE_KEY = "locus-display-settings";

const defaultFonts: Record<FontSlot, string> = {
  ui: "",
  prose: "",
  monoInline: "",
  monoBlock: "",
  monoEditor: "",
};

const defaults: DisplaySettings = {
  todoAutoOpen: true,
  changesAutoOpen: true,
  changesAutoClose: true,
  chatDiffReviewTarget: "inline",
  gitDiffReviewTarget: "inline",
  rightAlignUserMessages: true,
  compactToolCalls: true,
  hideThinkingBlocks: true,
  mergeGitTreeStatusIcon: true,
  hideGitCommandSuggestions: false,
  systemNotificationsEnabled: true,
  notifyOnChatDone: true,
  notifyOnSubagentDone: false,
  notifyOnAskUser: true,
  notifyOnChatError: true,
  notifyOnToolConfirm: true,
  fonts: { ...defaultFonts },
};

function load(): DisplaySettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return { ...defaults, ...parsed, fonts: { ...defaultFonts, ...parsed.fonts } };
    }
  } catch { /* ignore */ }
  return { ...defaults, fonts: { ...defaultFonts } };
}

function save(s: DisplaySettings) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(s)); } catch { /* ignore */ }
}

const state = reactive<DisplaySettings>(load());

export function useDisplaySettings() {
  function set<K extends keyof DisplaySettings>(key: K, value: DisplaySettings[K]) {
    state[key] = value;
    save({ ...state });
  }

  function setFont(slot: FontSlot, value: string) {
    state.fonts[slot] = value;
    save({ ...state, fonts: { ...state.fonts } });
    applyFonts(state.fonts);
  }

  return { state, set, setFont };
}

/* ---- Font CSS-variable application ---- */

const slotToCssVar: Record<FontSlot, string> = {
  ui: "--font-ui",
  prose: "--font-prose",
  monoInline: "--font-mono-inline",
  monoBlock: "--font-mono-block",
  monoEditor: "--font-mono-editor",
};

const slotToFallbackVar: Record<FontSlot, string> = {
  ui: "var(--font-stack-sans)",
  prose: "var(--font-stack-sans)",
  monoInline: "var(--font-stack-mono)",
  monoBlock: "var(--font-stack-mono)",
  monoEditor: "var(--font-stack-mono)",
};

/** Slots not exposed to UI but that should follow an exposed slot */
const aliasSlots: { cssVar: string; follows: FontSlot; fallback: string }[] = [
  { cssVar: "--font-mono-identifier", follows: "monoInline", fallback: "var(--font-stack-mono)" },
  { cssVar: "--font-mono-display",    follows: "monoEditor", fallback: "var(--font-stack-mono)" },
];

function applyFonts(fonts: Record<FontSlot, string>) {
  const root = document.documentElement;
  for (const slot of Object.keys(slotToCssVar) as FontSlot[]) {
    const custom = fonts[slot]?.trim();
    const cssVar = slotToCssVar[slot];
    if (custom) {
      root.style.setProperty(cssVar, `${custom}, ${slotToFallbackVar[slot]}`);
    } else {
      root.style.setProperty(cssVar, slotToFallbackVar[slot]);
    }
  }
  for (const alias of aliasSlots) {
    const custom = fonts[alias.follows]?.trim();
    if (custom) {
      root.style.setProperty(alias.cssVar, `${custom}, ${alias.fallback}`);
    } else {
      root.style.setProperty(alias.cssVar, alias.fallback);
    }
  }
}

/** Call once from App.vue to apply saved font overrides on startup */
export function initFonts() {
  applyFonts(state.fonts);
}

````

### src-tauri/src/session/gateway.rs

行数：292

````rust
use tauri::{AppHandle, Emitter};

use crate::commands::{StreamEvent, StreamEventEnvelope};
use crate::session::store::{
    SessionEventAppend, SessionEventMerge, SessionRunStatusUpdate, SessionStore,
};

const RUN_STATUS_RUNNING: &str = "running";
pub(crate) const RUN_STATUS_CANCELLING: &str = "cancelling";
const RUN_STATUS_WAITING_INPUT: &str = "waiting_input";
const RUN_STATUS_DONE: &str = "done";
const RUN_STATUS_CANCELLED: &str = "cancelled";
const RUN_STATUS_ERROR: &str = "error";

fn event_session_id(event: &StreamEvent) -> &str {
    match event {
        StreamEvent::RunStart { session_id }
        | StreamEvent::UserMessage { session_id, .. }
        | StreamEvent::TextDelta { session_id, .. }
        | StreamEvent::ThinkingDelta { session_id, .. }
        | StreamEvent::ToolCallStart { session_id, .. }
        | StreamEvent::ToolCallDone { session_id, .. }
        | StreamEvent::ToolCallDelta { session_id, .. }
        | StreamEvent::ToolCallProgress { session_id, .. }
        | StreamEvent::SubagentToolCallStart { session_id, .. }
        | StreamEvent::SubagentToolCallDone { session_id, .. }
        | StreamEvent::ToolCallRoundDone { session_id, .. }
        | StreamEvent::Done { session_id, .. }
        | StreamEvent::KnowledgeProposal { session_id, .. }
        | StreamEvent::UsageUpdate { session_id, .. }
        | StreamEvent::AskUser { session_id, .. }
        | StreamEvent::ToolConfirm { session_id, .. }
        | StreamEvent::InputAnswered { session_id, .. }
        | StreamEvent::UndoAvailable { session_id, .. }
        | StreamEvent::CompactStart { session_id, .. }
        | StreamEvent::CompactDone { session_id, .. }
        | StreamEvent::Cancelled { session_id, .. }
        | StreamEvent::Error { session_id, .. } => session_id,
    }
}

fn event_type(event: &StreamEvent) -> &'static str {
    match event {
        StreamEvent::RunStart { .. } => "runStart",
        StreamEvent::UserMessage { .. } => "userMessage",
        StreamEvent::TextDelta { .. } => "textDelta",
        StreamEvent::ThinkingDelta { .. } => "thinkingDelta",
        StreamEvent::ToolCallStart { .. } => "toolCallStart",
        StreamEvent::ToolCallDone { .. } => "toolCallDone",
        StreamEvent::ToolCallDelta { .. } => "toolCallDelta",
        StreamEvent::ToolCallProgress { .. } => "toolCallProgress",
        StreamEvent::SubagentToolCallStart { .. } => "subagentToolCallStart",
        StreamEvent::SubagentToolCallDone { .. } => "subagentToolCallDone",
        StreamEvent::ToolCallRoundDone { .. } => "toolCallRoundDone",
        StreamEvent::Done { .. } => "done",
        StreamEvent::KnowledgeProposal { .. } => "knowledgeProposal",
        StreamEvent::UsageUpdate { .. } => "usageUpdate",
        StreamEvent::AskUser { .. } => "askUser",
        StreamEvent::ToolConfirm { .. } => "toolConfirm",
        StreamEvent::InputAnswered { .. } => "inputAnswered",
        StreamEvent::UndoAvailable { .. } => "undoAvailable",
        StreamEvent::CompactStart { .. } => "compactStart",
        StreamEvent::CompactDone { .. } => "compactDone",
        StreamEvent::Cancelled { .. } => "cancelled",
        StreamEvent::Error { .. } => "error",
    }
}

fn run_status_for_event(event: &StreamEvent) -> Option<(&'static str, Option<String>)> {
    match event {
        StreamEvent::RunStart { .. }
        | StreamEvent::UserMessage { .. }
        | StreamEvent::TextDelta { .. }
        | StreamEvent::ThinkingDelta { .. }
        | StreamEvent::ToolCallStart { .. }
        | StreamEvent::ToolCallDone { .. }
        | StreamEvent::ToolCallDelta { .. }
        | StreamEvent::ToolCallProgress { .. }
        | StreamEvent::SubagentToolCallStart { .. }
        | StreamEvent::SubagentToolCallDone { .. }
        | StreamEvent::ToolCallRoundDone { .. }
        | StreamEvent::UsageUpdate { .. }
        | StreamEvent::InputAnswered { .. }
        | StreamEvent::UndoAvailable { .. }
        | StreamEvent::CompactStart { .. }
        | StreamEvent::CompactDone { .. } => Some((RUN_STATUS_RUNNING, None)),
        StreamEvent::AskUser { .. } | StreamEvent::ToolConfirm { .. } => {
            Some((RUN_STATUS_WAITING_INPUT, None))
        }
        StreamEvent::Done { .. } => Some((RUN_STATUS_DONE, None)),
        StreamEvent::Cancelled { .. } => Some((RUN_STATUS_CANCELLED, None)),
        StreamEvent::Error { error, .. } => Some((RUN_STATUS_ERROR, Some(error.message.clone()))),
        StreamEvent::KnowledgeProposal { .. } => None,
    }
}

fn is_terminal_run_status(status: &str) -> bool {
    matches!(
        status,
        RUN_STATUS_DONE | RUN_STATUS_CANCELLED | RUN_STATUS_ERROR
    )
}

fn event_merge(
    run_id: &str,
    session_id: &str,
    event_kind: &str,
    event: &StreamEvent,
) -> Option<SessionEventMerge> {
    match event {
        StreamEvent::TextDelta { text, .. } => Some(SessionEventMerge {
            key: format!("{}\u{0}{}\u{0}{}", session_id, run_id, event_kind),
            field: "text".to_string(),
            value: text.clone(),
        }),
        StreamEvent::ThinkingDelta { text, .. } => Some(SessionEventMerge {
            key: format!("{}\u{0}{}\u{0}{}", session_id, run_id, event_kind),
            field: "text".to_string(),
            value: text.clone(),
        }),
        StreamEvent::ToolCallDelta {
            tool_call_id,
            delta,
            ..
        } => Some(SessionEventMerge {
            key: format!(
                "{}\u{0}{}\u{0}{}\u{0}{}",
                session_id, run_id, event_kind, tool_call_id
            ),
            field: "delta".to_string(),
            value: delta.clone(),
        }),
        _ => None,
    }
}

#[cfg(debug_assertions)]
#[derive(Debug, PartialEq, Eq)]
enum RunSessionValidation {
    Match,
    Mismatch { run_session_id: String },
    UnknownRun,
}

#[cfg(debug_assertions)]
fn validate_run_session(
    store: &SessionStore,
    run_id: &str,
    event_session_id: &str,
) -> Result<RunSessionValidation, String> {
    match store.session_id_for_run(run_id)? {
        Some(run_session_id) if run_session_id == event_session_id => {
            Ok(RunSessionValidation::Match)
        }
        Some(run_session_id) => Ok(RunSessionValidation::Mismatch { run_session_id }),
        None => Ok(RunSessionValidation::UnknownRun),
    }
}

#[cfg(debug_assertions)]
fn warn_if_run_session_mismatch(
    store: &SessionStore,
    run_id: &str,
    event_session_id: &str,
    event_kind: &str,
) {
    match validate_run_session(store, run_id, event_session_id) {
        Ok(RunSessionValidation::Mismatch { run_session_id }) => {
            eprintln!(
                "[Locus] warning: stream event session/run mismatch: event={} event_session={} run={} run_session={}",
                event_kind, event_session_id, run_id, run_session_id
            );
        }
        Ok(RunSessionValidation::Match | RunSessionValidation::UnknownRun) => {}
        Err(error) => {
            eprintln!(
                "[Locus] warning: failed to validate stream event session/run ownership: event={} event_session={} run={} error={}",
                event_kind, event_session_id, run_id, error
            );
        }
    }
}

pub fn emit_stream(app_handle: &AppHandle, store: &SessionStore, run_id: &str, event: StreamEvent) {
    let session_id = event_session_id(&event).to_string();
    let event_kind = event_type(&event);
    #[cfg(debug_assertions)]
    warn_if_run_session_mismatch(store, run_id, &session_id, event_kind);

    let mut run_status =
        run_status_for_event(&event).map(|(status, error_message)| SessionRunStatusUpdate {
            run_id: run_id.to_string(),
            status: status.to_string(),
            error_message,
        });
    let merge = event_merge(run_id, &session_id, event_kind, &event);

    if run_status
        .as_ref()
        .is_some_and(|status| is_terminal_run_status(&status.status))
    {
        if let Some(status) = run_status.take() {
            if let Err(error) = store.update_run_status(
                &status.run_id,
                &status.status,
                status.error_message.as_deref(),
            ) {
                eprintln!(
                    "[Locus] failed to update terminal run status {} for session {} run {}: {}",
                    status.status, session_id, run_id, error
                );
            }
        }
    }

    let event_for_persist = event.clone();
    let _ = app_handle.emit(
        "stream-event",
        StreamEventEnvelope {
            run_id: run_id.to_string(),
            event,
        },
    );

    match serde_json::to_string(&event_for_persist) {
        Ok(payload_json) => {
            if let Err(error) = store.enqueue_session_event(
                SessionEventAppend {
                    session_id: session_id.clone(),
                    run_id: run_id.to_string(),
                    event_type: event_kind.to_string(),
                    payload_json,
                },
                merge,
                run_status,
            ) {
                eprintln!(
                    "[Locus] failed to queue session event {} for session {} run {}: {}",
                    event_kind, session_id, run_id, error
                );
            }
        }
        Err(error) => {
            eprintln!(
                "[Locus] failed to serialize session event {} for session {} run {}: {}",
                event_kind, session_id, run_id, error
            );
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use tempfile::tempdir;

    use super::{validate_run_session, RunSessionValidation};
    use crate::session::store::SessionStore;

    #[test]
    fn validate_run_session_detects_known_owner_mismatch() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let run_session_id = store
            .create_session("Run Owner", None, None, "chat", None)
            .expect("create run session");
        let other_session_id = store
            .create_session("Other", None, None, "chat", None)
            .expect("create other session");

        store
            .try_start_run(&run_session_id, "run-1")
            .expect("start run");

        assert_eq!(
            validate_run_session(&store, "run-1", &run_session_id).expect("validate matching run"),
            RunSessionValidation::Match
        );
        assert_eq!(
            validate_run_session(&store, "run-1", &other_session_id)
                .expect("validate mismatched run"),
            RunSessionValidation::Mismatch {
                run_session_id: run_session_id.clone()
            }
        );
        assert_eq!(
            validate_run_session(&store, "knowledge_1", &other_session_id)
                .expect("validate unknown run"),
            RunSessionValidation::UnknownRun
        );
    }
}

````

### src/__tests__/toolCallBatches.test.ts

行数：763

````ts
import { describe, expect, it } from "vitest";
import {
  areToolCallDisplaysCoveredByMatchState,
  buildMessageToolCalls,
  collectToolCallDisplayIds,
  collectToolCallDisplayIdMatchState,
  collectToolCallDisplayMatchState,
  filterToolCallsByConsumableMatchState,
  filterToolCallsByActiveIds,
  filterToolCallsByMatchState,
  firstToolCallRenderOrder,
  hasVisibleTextPartAfterToolCalls,
  lastToolCallRenderOrder,
  mergeSequentialAssistantToolCalls,
  mergeToolCallDisplaysWithoutDuplicates,
  resolveToolCallInfosForRender,
  splitToolCallsByRenderOrder,
  summarizeToolCallBatch,
} from "../composables/toolCallBatches";

import type { ChatMessage, ToolCallDisplay, ToolCallInfo } from "../types";

function makeToolCall(status: ToolCallDisplay["status"], id: string): ToolCallDisplay {
  return {
    id,
    name: "read",
    arguments: "{}",
    status,
  };
}

function makeToolInfo(id: string, name = "read"): ToolCallInfo {
  return {
    id,
    name,
    arguments: "{}",
  };
}

describe("toolCallBatches", () => {
  it("builds historical message tool calls with persisted output fallback", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-1",
          name: "web_search",
          arguments: "{\"q\":\"unity\"}",
        },
      ],
    };

    expect(buildMessageToolCalls(message, { "tc-1": "cached output" })).toEqual([
      {
        id: "tc-1",
        name: "web_search",
        arguments: "{\"q\":\"unity\"}",
        status: "done",
        output: "cached output",
      },
    ]);
  });

  it("prefers server tool output for historical message tool calls", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-2",
          name: "web_search",
          arguments: "{\"q\":\"agent\"}",
          serverToolOutput: "server output",
        },
      ],
    };

    expect(buildMessageToolCalls(message, { "tc-2": "cached output" })[0]?.output).toBe("server output");
  });

  it("preserves persisted render order for historical message tool calls", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-ordered",
          name: "read",
          arguments: "{}",
          order: 4,
        },
      ],
    };

    expect(buildMessageToolCalls(message, {})[0]?.order).toBe(4);
  });

  it("preserves persisted outcomes for historical tool calls", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-3",
          name: "write",
          arguments: "{\"path\":\"Assets/Player.cs\"}",
          outcome: "error",
        },
      ],
    };

    expect(buildMessageToolCalls(message, {})[0]?.status).toBe("error");
  });

  it("restores nested tool calls and recorded output from persisted history", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "task-1",
          name: "task",
          arguments: "{}",
          outcome: "done",
          nestedToolCalls: [
            {
              id: "read-1",
              name: "read",
              arguments: "{\"path\":\"Assets/Player.cs\"}",
              outcome: "done",
              recordedOutput: "class Player {}",
            },
          ],
        },
      ],
    };

    const toolCalls = buildMessageToolCalls(message, {});
    expect(toolCalls[0]?.nestedToolCalls?.[0]?.output).toBe("class Player {}");
    expect(toolCalls[0]?.nestedToolCalls?.[0]?.status).toBe("done");
  });

  it("uses the earliest nested persisted order for a tool block", () => {
    const toolCalls: ToolCallDisplay[] = [
      {
        id: "task-1",
        name: "task",
        arguments: "{}",
        status: "done",
        nestedToolCalls: [
          {
            id: "read-1",
            name: "read",
            arguments: "{}",
            status: "done",
            order: 2,
          },
        ],
      },
    ];

    expect(firstToolCallRenderOrder(toolCalls)).toBe(2);
  });

  it("uses the latest nested persisted order for handoff collapse boundaries", () => {
    const toolCalls: ToolCallDisplay[] = [
      {
        id: "task-1",
        name: "task",
        arguments: "{}",
        status: "done",
        order: 3,
        nestedToolCalls: [
          {
            id: "read-1",
            name: "read",
            arguments: "{}",
            status: "done",
            order: 5,
          },
        ],
      },
      {
        id: "grep-1",
        name: "grep",
        arguments: "{}",
        status: "done",
        order: 4,
      },
    ];

    expect(lastToolCallRenderOrder(toolCalls)).toBe(5);
  });

  it("detects only body text rendered after the handoff tools", () => {
    const toolCalls: ToolCallDisplay[] = [
      { ...makeToolCall("done", "tc-1"), order: 3 },
      { ...makeToolCall("done", "tc-2"), order: 4 },
    ];

    expect(hasVisibleTextPartAfterToolCalls([
      {
        kind: "text",
        id: "before",
        order: { runId: "run-1", seq: 2 },
        content: "工具前的正文",
      },
    ], toolCalls)).toBe(false);

    expect(hasVisibleTextPartAfterToolCalls([
      {
        kind: "text",
        id: "after",
        order: { runId: "run-1", seq: 5 },
        content: "工具后的正文",
      },
    ], toolCalls)).toBe(true);
  });

  it("splits ordered tool groups around non-tool render boundaries", () => {
    const segments = splitToolCallsByRenderOrder(
      [
        { ...makeToolCall("done", "tc-before"), order: 2 },
        { ...makeToolCall("done", "tc-after-a"), order: 4 },
        { ...makeToolCall("done", "tc-after-b"), order: 5 },
      ],
      { fallbackOrder: 20, boundaryOrders: [3] },
    );

    expect(segments.map((segment) => segment.toolCalls.map((toolCall) => toolCall.id))).toEqual([
      ["tc-before"],
      ["tc-after-a", "tc-after-b"],
    ]);
  });

  it("collects nested active tool call ids for transcript de-duplication", () => {
    const activeToolCalls: ToolCallDisplay[] = [
      {
        id: "task-1",
        name: "task",
        arguments: "{}",
        status: "running",
        nestedToolCalls: [
          {
            id: "read-1",
            name: "read",
            arguments: "{}",
            status: "done",
          },
        ],
      },
    ];

    expect(Array.from(collectToolCallDisplayIds(activeToolCalls))).toEqual(["task-1", "read-1"]);
  });

  it("filters historical tool calls that are still present in active tool calls", () => {
    const filtered = filterToolCallsByActiveIds(
      [
        makeToolInfo("task-1", "task"),
        makeToolInfo("read-1"),
        makeToolInfo("grep-1", "grep"),
      ],
      new Set(["task-1", "read-1"]),
    );

    expect(filtered).toEqual([makeToolInfo("grep-1", "grep")]);
  });

  it("drops historical tool-only rounds after filtering when the active copy is already visible", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: filterToolCallsByActiveIds(
          [makeToolInfo("task-1", "task")],
          new Set(["task-1"]),
        ),
      },
      {
        id: "m2",
        content: "继续处理后续文件。",
      },
    ]);

    expect(merged).toHaveLength(2);
    expect(merged[0]?.displayToolCalls).toBeUndefined();
    expect(merged[1]?.displayToolCalls).toBeUndefined();
  });

  it("keeps resolved empty display tool calls hidden instead of falling back to persisted history", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: filterToolCallsByMatchState(
          [makeToolInfo("task-1", "task")],
          collectToolCallDisplayMatchState([makeToolCall("done", "task-1")]),
        ),
      },
    ]);

    expect(
      resolveToolCallInfosForRender({
        messageToolCalls: [makeToolInfo("task-1", "task")],
        displayToolCalls: merged[0]?.displayToolCalls,
      }),
    ).toBeUndefined();
  });

  it("falls back to persisted history only when no resolved display tool calls exist", () => {
    expect(
      resolveToolCallInfosForRender({
        messageToolCalls: [makeToolInfo("task-1", "task")],
      }),
    ).toEqual([makeToolInfo("task-1", "task")]);
  });

  it("filters historical tool calls that match the transient copy even when ids differ", () => {
    const hiddenState = collectToolCallDisplayMatchState([
      {
        id: "active-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
    ]);

    const filtered = filterToolCallsByMatchState(
      [
        {
          id: "history-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        },
        {
          id: "history-2",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoB.cs\"}",
        },
      ],
      hiddenState,
    );

    expect(filtered).toEqual([
      {
        id: "history-2",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoB.cs\"}",
      },
    ]);
  });

  it("keeps prior identical tools visible while a new active tool is running", () => {
    const hiddenState = collectToolCallDisplayIdMatchState([
      {
        id: "active-recompile",
        name: "unity_recompile",
        arguments: "{\"project_path\":\"F:/Unity/Game\",\"editor_status\":\"editing\"}",
        status: "running",
      },
    ]);

    const priorRecompile = {
      id: "history-recompile",
      name: "unity_recompile",
      arguments: "{\"project_path\":\"F:/Unity/Game\",\"editor_status\":\"editing\"}",
    };

    expect(filterToolCallsByConsumableMatchState([priorRecompile], hiddenState)).toEqual([
      priorRecompile,
    ]);
    expect(filterToolCallsByConsumableMatchState([
      {
        ...priorRecompile,
        id: "active-recompile",
      },
    ], hiddenState)).toBeUndefined();
  });

  it("consumes transient semantic matches once across a transcript tail", () => {
    const hiddenState = collectToolCallDisplayMatchState([
      {
        id: "active-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
    ]);
    const repeatedHistoryCall = {
      id: "history-1",
      name: "read",
      arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
    };

    expect(filterToolCallsByConsumableMatchState([repeatedHistoryCall], hiddenState)).toBeUndefined();
    expect(filterToolCallsByConsumableMatchState([
      {
        ...repeatedHistoryCall,
        id: "history-2",
      },
    ], hiddenState)).toEqual([
      {
        ...repeatedHistoryCall,
        id: "history-2",
      },
    ]);
  });

  it("consumes an id match together with its semantic fingerprint", () => {
    const hiddenState = collectToolCallDisplayMatchState([
      {
        id: "tc-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
    ]);

    expect(filterToolCallsByConsumableMatchState([
      {
        id: "tc-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
      },
    ], hiddenState)).toBeUndefined();
    expect(filterToolCallsByConsumableMatchState([
      {
        id: "history-2",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
      },
    ], hiddenState)).toEqual([
      {
        id: "history-2",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
      },
    ]);
  });

  it("merges transient and promoted history tool calls without duplicating semantic matches", () => {
    const merged = mergeToolCallDisplaysWithoutDuplicates(
      [
        {
          id: "history-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
          status: "done",
        },
      ],
      [
        {
          id: "active-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
          status: "done",
        },
        {
          id: "active-2",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoB.cs\"}",
          status: "done",
        },
      ],
    );

    expect(merged.map((toolCall) => toolCall.id)).toEqual(["history-1", "active-2"]);
  });

  it("deduplicates read tool calls when path aliases differ across transient and history copies", () => {
    const filtered = filterToolCallsByMatchState(
      [
        {
          id: "history-1",
          name: "read",
          arguments: "{\"filePath\":\"Assets\\\\Scripts\\\\TestMonoA.cs\"}",
        },
      ],
      collectToolCallDisplayMatchState([
        {
          id: "active-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
          status: "done",
        },
      ]),
    );

    expect(filtered).toBeUndefined();
  });

  it("deduplicates edit tool calls when camelCase and snake_case aliases mix", () => {
    const merged = mergeToolCallDisplaysWithoutDuplicates(
      [
        {
          id: "history-1",
          name: "edit",
          arguments: "{\"file_path\":\"Assets/Test.cs\",\"old_string\":\"a\",\"new_string\":\"b\",\"replace_all\":true}",
          status: "done",
        },
      ],
      [
        {
          id: "active-1",
          name: "edit",
          arguments: "{\"filePath\":\"Assets/Test.cs\",\"oldString\":\"a\",\"newString\":\"b\",\"replaceAll\":true}",
          status: "done",
        },
      ],
    );

    expect(merged.map((toolCall) => toolCall.id)).toEqual(["history-1"]);
  });

  it("checks whether a historical tool segment is fully covered by a retained match state", () => {
    const retainedState = collectToolCallDisplayMatchState([
      {
        id: "handoff-read",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
      {
        id: "handoff-grep",
        name: "grep",
        arguments: "{\"path\":\"Assets/Scripts\"}",
        status: "done",
      },
    ]);

    expect(areToolCallDisplaysCoveredByMatchState([
      {
        id: "history-read",
        name: "read",
        arguments: "{\"filePath\":\"Assets\\\\Scripts\\\\TestMonoA.cs\"}",
        status: "done",
      },
      {
        id: "history-grep",
        name: "grep",
        arguments: "{\"path\":\"Assets/Scripts\"}",
        status: "done",
      },
    ], retainedState)).toBe(true);

    expect(areToolCallDisplaysCoveredByMatchState([
      {
        id: "history-read",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
      {
        id: "history-extra",
        name: "list",
        arguments: "{\"path\":\"Assets\"}",
        status: "done",
      },
    ], retainedState)).toBe(false);
  });

  it("collapses completed tool batches when compact mode is enabled", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("done", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(true);
    expect(batch.total).toBe(2);
    expect(batch.doneCount).toBe(2);
  });

  it("keeps completed tool batches expanded when compact mode is disabled", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("done", "tc-2")],
      false,
    );

    expect(batch.canCollapse).toBe(false);
  });

  it("keeps single tool batches expanded", () => {
    const batch = summarizeToolCallBatch([makeToolCall("done", "tc-1")], true);

    expect(batch.canCollapse).toBe(false);
  });

  it("keeps running streaming tool batches expanded", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("running", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(false);
    expect(batch.runningCount).toBe(1);
  });

  it("collapses terminal tool batches with failed calls", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("error", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(true);
    expect(batch.errorCount).toBe(1);
  });

  it("collapses terminal tool batches with interrupted calls", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("interrupted", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(true);
    expect(batch.interruptedCount).toBe(1);
  });

  it("applies the same rule to nested tool batches", () => {
    const parent: ToolCallDisplay = {
      id: "parent",
      name: "task",
      arguments: "{}",
      status: "done",
      nestedToolCalls: [makeToolCall("done", "tc-1"), makeToolCall("done", "tc-2")],
    };

    const batch = summarizeToolCallBatch(parent.nestedToolCalls ?? [], true);

    expect(batch.canCollapse).toBe(true);
  });

  it("merges tool-only assistant rounds into the following visible round", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-ask", "ask_user_question")],
      },
      {
        id: "m2",
        content: "",
        toolCalls: [makeToolInfo("tc-list", "list")],
      },
      {
        id: "m3",
        content: "已并行读取项目目录下 4 个现有文件。",
        toolCalls: [
          makeToolInfo("tc-read-1"),
          makeToolInfo("tc-read-2"),
          makeToolInfo("tc-read-3"),
          makeToolInfo("tc-read-4"),
        ],
      },
    ]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.id).toBe("m3");
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual([
      "tc-ask",
      "tc-list",
      "tc-read-1",
      "tc-read-2",
      "tc-read-3",
      "tc-read-4",
    ]);
    expect(merged[0]?.displayToolCallsBeforeContent?.map((toolCall) => toolCall.id)).toEqual([
      "tc-ask",
      "tc-list",
    ]);
    expect(merged[0]?.displayToolCallsAfterContent?.map((toolCall) => toolCall.id)).toEqual([
      "tc-read-1",
      "tc-read-2",
      "tc-read-3",
      "tc-read-4",
    ]);
  });

  it("keeps visible assistant rounds separate", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "先确认范围。",
        toolCalls: [makeToolInfo("tc-ask", "ask_user_question")],
      },
      {
        id: "m2",
        content: "目录里只有 4 个文件。",
        toolCalls: [makeToolInfo("tc-list", "list")],
      },
    ]);

    expect(merged).toHaveLength(2);
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-ask"]);
    expect(merged[1]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-list"]);
    expect(merged[0]?.displayToolCallsAfterContent?.map((toolCall) => toolCall.id)).toEqual(["tc-ask"]);
    expect(merged[1]?.displayToolCallsAfterContent?.map((toolCall) => toolCall.id)).toEqual(["tc-list"]);
  });

  it("keeps trailing tool-only assistant rounds in one append-only list before text arrives", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-1")],
      },
      {
        id: "m2",
        content: "",
        toolCalls: [makeToolInfo("tc-2")],
      },
      {
        id: "m3",
        content: "",
        toolCalls: [makeToolInfo("tc-3")],
      },
    ]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.id).toBe("m1");
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-1", "tc-2", "tc-3"]);
  });

  it("keeps proposal-bearing tool rounds on their own render item", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-proposal", "knowledge_edit")],
        attachedKnowledgeProposalCount: 1,
      },
      {
        id: "m2",
        content: "",
        toolCalls: [makeToolInfo("tc-read")],
      },
    ]);

    expect(merged).toHaveLength(2);
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-proposal"]);
    expect(merged[1]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-read"]);
  });

  it("keeps thinking-only tool rounds separate so their thinking block can render", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-1")],
      },
      {
        id: "m2",
        content: "",
        thinkingContent: "thinking",
        toolCalls: [makeToolInfo("tc-2")],
      },
      {
        id: "m3",
        content: "最终答复",
      },
    ]);

    expect(merged).toHaveLength(3);
    expect(merged[0]?.id).toBe("m1");
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-1"]);
    expect(merged[1]?.id).toBe("m2");
    expect(merged[1]?.thinkingContent).toBe("thinking");
    expect(merged[1]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-2"]);
    expect(merged[2]?.id).toBe("m3");
  });
});

````

### src/__tests__/chatViewStability.test.ts

行数：349

````ts
import { describe, expect, it, vi } from "vitest";
import {
  collectPendingContinuationToolItemIds,
  collectPendingContinuationToolSegmentItemIds,
  createCoalescedScrollScheduler,
  createSettledScrollScheduler,
  findTrailingAssistantToolMessageId,
  hasRunningToolCall,
  shouldAutoScrollToBottom,
  shouldShowAssistantContinuation,
  shouldShowWaitingPlaceholder,
} from "../composables/chatViewStability";

type TestFrameCallback = (time: number) => void;

describe("chatViewStability", () => {
  it("shows assistant continuation only while a transient assistant block exists", () => {
    expect(shouldShowAssistantContinuation("assistant", true)).toBe(true);
    expect(shouldShowAssistantContinuation("assistant", false)).toBe(false);
    expect(shouldShowAssistantContinuation("user", true)).toBe(false);
    expect(shouldShowAssistantContinuation(null, true)).toBe(false);
  });

  it("keeps trailing tool-only assistant rounds expanded while the run is still streaming", () => {
    const pendingIds = collectPendingContinuationToolItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      items: [
        { id: "m1", content: "已读取项目结构。", toolCallCount: 2 },
        { id: "m2", content: "", toolCallCount: 3 },
        { id: "m3", content: "", toolCallCount: 2 },
      ],
    });

    expect(Array.from(pendingIds)).toEqual(["m3", "m2"]);
  });

  it("keeps the tool segment after the latest body expanded until another body arrives", () => {
    const pendingIds = collectPendingContinuationToolSegmentItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      segments: [
        { type: "content" },
        { type: "toolCalls", itemIds: ["m1"] },
        { type: "content" },
        { type: "toolCalls", itemIds: ["m2"] },
      ],
    });

    expect(Array.from(pendingIds)).toEqual(["m2"]);
  });

  it("keeps trailing tool segments expanded across non-body assistant parts", () => {
    const pendingIds = collectPendingContinuationToolSegmentItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      segments: [
        { type: "content" },
        { type: "toolCalls", itemIds: ["m2"] },
        { type: "other" },
      ],
    });

    expect(Array.from(pendingIds)).toEqual(["m2"]);
  });

  it("stops pinning tool segments once a later body exists", () => {
    const pendingIds = collectPendingContinuationToolSegmentItemIds({
      isStreaming: true,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: true,
      segments: [
        { type: "content" },
        { type: "toolCalls", itemIds: ["m2"] },
        { type: "content" },
      ],
    });

    expect(Array.from(pendingIds)).toEqual([]);
  });

  it("stops pinning historical tool rounds once the final response arrives", () => {
    const pendingIds = collectPendingContinuationToolItemIds({
      isStreaming: false,
      lastGroupRole: "assistant",
      hasTransientAssistantMessage: false,
      items: [
        { id: "m1", content: "", toolCallCount: 3 },
        { id: "m2", content: "", toolCallCount: 2 },
      ],
    });

    expect(Array.from(pendingIds)).toEqual([]);
  });

  it("pins only the latest visible assistant message when it contains tool calls", () => {
    expect(findTrailingAssistantToolMessageId([
      {
        id: "a1",
        role: "assistant",
        toolCalls: [{ id: "tc-1", name: "read_file", arguments: "{\"path\":\"Assets/Player.cs\"}" }],
      },
      {
        id: "tool-1",
        role: "tool",
        content: "ok",
      } as any,
    ])).toBe("a1");

    expect(findTrailingAssistantToolMessageId([
      {
        id: "a1",
        role: "assistant",
        toolCalls: [{ id: "tc-1", name: "read_file", arguments: "{\"path\":\"Assets/Player.cs\"}" }],
      },
      {
        id: "u1",
        role: "user",
        content: "继续",
      } as any,
    ])).toBeNull();

    expect(findTrailingAssistantToolMessageId([
      {
        id: "a2",
        role: "assistant",
        content: "这里是最终答复",
      } as any,
    ])).toBeNull();
  });

  it("shows the waiting placeholder whenever the run is idle between stream events", () => {
    expect(shouldShowWaitingPlaceholder({
      isStreaming: true,
      hasStreamingContent: false,
      isThinking: false,
      hasThinkingContent: false,
    })).toBe(true);

    expect(shouldShowWaitingPlaceholder({
      isStreaming: true,
      hasStreamingContent: false,
      isThinking: false,
      hasThinkingContent: false,
    })).toBe(true);

    expect(shouldShowWaitingPlaceholder({
      isStreaming: true,
      hasStreamingContent: true,
      isThinking: false,
      hasThinkingContent: false,
    })).toBe(false);
  });

  it("treats completed tool calls as idle while nested running tools stay active", () => {
    expect(hasRunningToolCall([
      { status: "done" },
      { status: "error" },
      { status: "interrupted" },
    ])).toBe(false);

    expect(hasRunningToolCall([
      {
        status: "done",
        nestedToolCalls: [
          { id: "nested-1", name: "read", arguments: "{}", status: "running" },
        ],
      },
    ])).toBe(true);
  });

  it("skips auto-scroll when the user is reviewing older content", () => {
    expect(shouldAutoScrollToBottom({
      force: false,
      remembered: { mode: "offset", scrollTop: 240 },
      metrics: {
        scrollTop: 240,
        clientHeight: 300,
        scrollHeight: 1200,
      },
    })).toBe(false);

    expect(shouldAutoScrollToBottom({
      force: false,
      remembered: {
        mode: "anchor",
        anchorId: "m2",
        offsetTop: 20,
        fallbackScrollTop: 240,
      },
      metrics: {
        scrollTop: 240,
        clientHeight: 300,
        scrollHeight: 1200,
      },
    })).toBe(false);

    expect(shouldAutoScrollToBottom({
      force: true,
      remembered: { mode: "offset", scrollTop: 240 },
      metrics: {
        scrollTop: 240,
        clientHeight: 300,
        scrollHeight: 1200,
      },
    })).toBe(true);
  });

  it("coalesces repeated scroll requests into one frame and preserves force", () => {
    const calls: boolean[] = [];
    let scheduledFrame: TestFrameCallback | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 1;
    });
    const cancelFrame = vi.fn();

    const scheduler = createCoalescedScrollScheduler(
      (force) => calls.push(force),
      requestFrame,
      cancelFrame,
    );

    scheduler.schedule(false);
    scheduler.schedule(false);
    scheduler.schedule(true);

    expect(requestFrame).toHaveBeenCalledTimes(1);
    expect(calls).toEqual([]);

    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }

    expect(calls).toEqual([true]);
  });

  it("cancels pending scroll work cleanly", () => {
    let scheduledFrame: TestFrameCallback | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 7;
    });
    const cancelFrame = vi.fn();
    const run = vi.fn();

    const scheduler = createCoalescedScrollScheduler(run, requestFrame, cancelFrame);

    scheduler.schedule();
    scheduler.cancel();
    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }

    expect(cancelFrame).toHaveBeenCalledWith(7);
    expect(run).not.toHaveBeenCalled();
  });

  it("retries scroll work after the next frame and after layout settles", () => {
    const calls: number[] = [];
    let scheduledFrame: TestFrameCallback | null = null;
    let scheduledTimeout: (() => void) | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 3;
    });
    const cancelFrame = vi.fn();
    const requestTimeout = vi.fn((cb: () => void, delay: number) => {
      scheduledTimeout = cb;
      expect(delay).toBe(320);
      return 9;
    });
    const cancelTimeout = vi.fn();

    const scheduler = createSettledScrollScheduler(
      () => calls.push(calls.length),
      320,
      requestFrame,
      cancelFrame,
      requestTimeout,
      cancelTimeout,
    );

    scheduler.schedule();

    expect(calls).toEqual([0]);
    expect(requestFrame).toHaveBeenCalledTimes(1);
    expect(requestTimeout).toHaveBeenCalledTimes(1);

    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }
    expect(calls).toEqual([0, 1]);

    const flushTimeout = scheduledTimeout as (() => void) | null;
    if (flushTimeout) {
      flushTimeout();
    }
    expect(calls).toEqual([0, 1, 2]);
  });

  it("cancels pending settled scroll retries", () => {
    const run = vi.fn();
    let scheduledFrame: TestFrameCallback | null = null;
    let scheduledTimeout: (() => void) | null = null;
    const requestFrame = vi.fn((cb: TestFrameCallback) => {
      scheduledFrame = cb;
      return 11;
    });
    const cancelFrame = vi.fn();
    const requestTimeout = vi.fn((cb: () => void) => {
      scheduledTimeout = cb;
      return 13;
    });
    const cancelTimeout = vi.fn();

    const scheduler = createSettledScrollScheduler(
      run,
      320,
      requestFrame,
      cancelFrame,
      requestTimeout,
      cancelTimeout,
    );

    scheduler.schedule();
    scheduler.cancel();

    const flushFrame = scheduledFrame as TestFrameCallback | null;
    if (flushFrame) {
      flushFrame(0);
    }
    const flushTimeout = scheduledTimeout as (() => void) | null;
    if (flushTimeout) {
      flushTimeout();
    }

    expect(run).toHaveBeenCalledTimes(1);
    expect(cancelFrame).toHaveBeenCalledWith(11);
    expect(cancelTimeout).toHaveBeenCalledWith(13);
  });
});

````

### src/__tests__/useStreamReducer.test.ts

行数：1192

````ts
import { describe, it, expect } from "vitest";
import { buildToolResultMessages, mergeUserMessage, reduceStreamEvent, type StreamState } from "../composables/useStreamReducer";
import type { StreamEvent, ToolCallDisplay } from "../types";

function makeState(overrides?: Partial<StreamState>): StreamState {
  return {
    messages: [],
    streamingText: "",
    rawStreamText: "",
    streamingThinking: "",
    streamSequence: 0,
    streamingTextOrder: 0,
    thinkingOrder: 0,
    liveRenderParts: [],
    isStreaming: false,
    isCompacting: false,
    isThinking: false,
    thinkingStartTime: 0,
    thinkingDuration: 0,
    activeToolCalls: [],
    tokenUsage: {
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCacheReadTokens: 0,
      totalCacheWriteTokens: 0,
      totalCostUsd: 0,
      pricedRounds: 0,
      contextTokens: 0,
      contextLimit: 0,
    },
    todos: [],
    showTodoPanel: false,
    pendingQuestion: null,
    pendingToolConfirms: [],
    undoableMessageIds: new Set(),
    ...overrides,
  };
}

describe("reduceStreamEvent", () => {
  describe("buildToolResultMessages", () => {
    it("materializes completed tool outputs as hidden tool result messages", () => {
      expect(buildToolResultMessages([
        {
          id: "tc-1",
          name: "read",
          arguments: "{}",
          status: "done",
          output: "file contents",
        },
        {
          id: "tc-2",
          name: "grep",
          arguments: "{}",
          status: "running",
        },
      ], 123)).toEqual([
        {
          id: "tool_result_tc-1",
          role: "tool",
          content: "file contents",
          createdAt: 123,
          toolCallId: "tc-1",
        },
      ]);
    });
  });

  describe("textDelta", () => {
    it("marks the first visible text with a stream order", () => {
      const state = makeState({ isStreaming: true, streamSequence: 2 });
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "hello" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "setStreamingTextOrder", order: 3 });
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 3 });
    });

    it("uses backend render order for the first visible text", () => {
      const state = makeState({ isStreaming: true, streamSequence: 2 });
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "hello", order: 7 };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "setStreamingTextOrder", order: 7 });
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 7 });
    });

    it("normalizes repeated backend render order after prior stream items", () => {
      const state = makeState({ isStreaming: true, streamSequence: 7 });
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "next", order: 1 };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "setStreamingTextOrder", order: 8 });
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 8 });
    });

    it("appends text without auto-activating streaming (streaming controlled by chat store)", () => {
      const state = makeState();
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "hello" };
      const mutations = reduceStreamEvent(state, event);

      // Auto-activation removed — streaming now controlled by runStart/cancel
      expect(mutations.filter((m) => m.type === "setStreaming")).toHaveLength(0);
      expect(mutations).toContainEqual({ type: "appendRawText", text: "hello" });
      expect(mutations).toContainEqual({ type: "setThinking", value: false });
    });

    it("does not set streaming state on text delta", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "world" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.filter((m) => m.type === "setStreaming")).toHaveLength(0);
    });

    it("updates thinking duration if thinking was active", () => {
      const state = makeState({ isStreaming: true, isThinking: true, thinkingStartTime: Date.now() - 5000 });
      const event: StreamEvent = { runId: "test-run", type: "textDelta", sessionId: "s1", text: "x" };
      const mutations = reduceStreamEvent(state, event);

      const durationMut = mutations.find((m) => m.type === "updateThinkingDuration");
      expect(durationMut).toBeDefined();
      if (durationMut?.type === "updateThinkingDuration") {
        expect(durationMut.duration).toBeGreaterThanOrEqual(4);
        expect(durationMut.duration).toBeLessThanOrEqual(6);
      }
    });
  });

  describe("thinkingDelta", () => {
    it("marks the first thinking block with a stream order", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "thinking..." };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "setThinkingOrder", order: 1 });
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 1 });
    });

    it("uses backend render order for the first thinking block", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "thinking...", order: 4 };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "setThinkingOrder", order: 4 });
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 4 });
    });

    it("appends thinking and starts thinking mode", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "thinking..." };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "appendThinking", text: "thinking..." });
      const setThinking = mutations.find((m) => m.type === "setThinking");
      expect(setThinking).toBeDefined();
      if (setThinking?.type === "setThinking") {
        expect(setThinking.value).toBe(true);
        expect(setThinking.startTime).toBeGreaterThan(0);
      }
    });

    it("does not reset thinking start if already thinking", () => {
      const state = makeState({ isStreaming: true, isThinking: true, thinkingStartTime: 1000 });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "more" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "appendThinking", text: "more" });
      expect(mutations.find((m) => m.type === "setThinking")).toBeUndefined();
    });

    it("starts a later thinking block after tools have started", () => {
      const state = makeState({
        isStreaming: true,
        streamSequence: 1,
        activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running", order: 1 }],
      });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "late" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "setThinkingOrder", order: 2 });
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 2 });
      expect(mutations).toContainEqual({ type: "appendThinking", text: "late" });
      const setThinking = mutations.find((m) => m.type === "setThinking");
      expect(setThinking).toBeDefined();
      if (setThinking?.type === "setThinking") {
        expect(setThinking.value).toBe(true);
        expect(setThinking.startTime).toBeGreaterThan(0);
      }
    });

    it("keeps an already active thinking block when tools are present", () => {
      const state = makeState({
        isStreaming: true,
        isThinking: true,
        thinkingStartTime: Date.now() - 3000,
        activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running" }],
      });
      const event: StreamEvent = { runId: "test-run", type: "thinkingDelta", sessionId: "s1", text: "late" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "appendThinking", text: "late" });
      expect(mutations.find((m) => m.type === "setThinking")).toBeUndefined();
      expect(mutations.find((m) => m.type === "updateThinkingDuration")).toBeUndefined();
    });
  });

  describe("toolCallStart", () => {
    it("marks top-level tool calls with stream order", () => {
      const state = makeState({ isStreaming: true, streamSequence: 1 });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        arguments: '{"path":"foo.ts"}',
      };
      const mutations = reduceStreamEvent(state, event);
      const addMut = mutations.find((m) => m.type === "addToolCall");

      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 2 });
      expect(addMut?.type === "addToolCall" ? addMut.toolCall.order : 0).toBe(2);
    });

    it("uses backend render order for top-level tool calls", () => {
      const state = makeState({ isStreaming: true, streamSequence: 1 });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        arguments: '{"path":"foo.ts"}',
        order: 6,
      };
      const mutations = reduceStreamEvent(state, event);
      const addMut = mutations.find((m) => m.type === "addToolCall");

      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 6 });
      expect(addMut?.type === "addToolCall" ? addMut.toolCall.order : 0).toBe(6);
    });

    it("normalizes repeated backend render order for later tool rounds", () => {
      const state = makeState({ isStreaming: true, streamSequence: 8 });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc-later",
        toolName: "read",
        arguments: "{}",
        order: 2,
      };
      const mutations = reduceStreamEvent(state, event);
      const addMut = mutations.find((m) => m.type === "addToolCall");

      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 9 });
      expect(addMut?.type === "addToolCall" ? addMut.toolCall.order : 0).toBe(9);
    });

    it("adds a new tool call", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        arguments: '{"path":"foo.ts"}',
      };
      const mutations = reduceStreamEvent(state, event);

      const addMut = mutations.find((m) => m.type === "addToolCall");
      expect(addMut).toBeDefined();
      if (addMut?.type === "addToolCall") {
        expect(addMut.toolCall.id).toBe("tc1");
        expect(addMut.toolCall.name).toBe("read");
        expect(addMut.toolCall.status).toBe("running");
      }
    });

    it("closes active thinking before adding a tool call", () => {
      const state = makeState({ isStreaming: true, isThinking: true, thinkingStartTime: Date.now() - 4000 });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        arguments: '{"path":"foo.ts"}',
      };
      const mutations = reduceStreamEvent(state, event);
      const updateIndex = mutations.findIndex((m) => m.type === "updateThinkingDuration");
      const stopIndex = mutations.findIndex((m) => m.type === "setThinking");
      const addIndex = mutations.findIndex((m) => m.type === "addToolCall");

      expect(updateIndex).toBeGreaterThanOrEqual(0);
      expect(stopIndex).toBeGreaterThan(updateIndex);
      expect(addIndex).toBeGreaterThan(stopIndex);
      expect(mutations[stopIndex]).toEqual({ type: "setThinking", value: false });
    });

    it("updates existing tool call arguments", () => {
      const existingTc: ToolCallDisplay = { id: "tc1", name: "read", arguments: "{}", status: "running" };
      const state = makeState({ isStreaming: true, activeToolCalls: [existingTc] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallStart",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        arguments: '{"path":"bar.ts"}',
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.find((m) => m.type === "addToolCall")).toBeUndefined();
      const update = mutations.find((m) => m.type === "updateToolCall");
      expect(update).toBeDefined();
      if (update?.type === "updateToolCall") {
        expect(update.id).toBe("tc1");
        expect(update.updates.arguments).toBe('{"path":"bar.ts"}');
        expect(update.updates.order).toBe(1);
      }
    });
  });

  describe("toolCallDone", () => {
    it("marks tool call as done", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running" }] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        output: "file contents",
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateToolCall",
        id: "tc1",
        updates: { status: "done", output: "file contents", progress: null },
      });
    });

    it("marks tool call as error when outcome is error", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running" }] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        output: "not found",
        outcome: "error",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateToolCall",
        id: "tc1",
        updates: { status: "error", output: "not found", progress: null },
      });
    });

    it("marks tool call as interrupted when outcome is interrupted", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "read", arguments: "{}", status: "running" }] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "read",
        output: "工具执行被用户中止，未返回结果。",
        outcome: "interrupted",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateToolCall",
        id: "tc1",
        updates: { status: "interrupted", output: "工具执行被用户中止，未返回结果。", progress: null },
      });
    });

    it("parses todowrite output", () => {
      const state = makeState({ isStreaming: true, activeToolCalls: [{ id: "tc1", name: "todowrite", arguments: "{}", status: "running" }] });
      const todos = [{ content: "do thing", status: "pending", priority: "medium" }];
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "todowrite",
        output: `Todos updated: ${JSON.stringify(todos)}`,
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      const todoMut = mutations.find((m) => m.type === "setTodos");
      expect(todoMut).toBeDefined();
      if (todoMut?.type === "setTodos") {
        expect(todoMut.runId).toBe("test-run");
        expect(todoMut.todos).toHaveLength(1);
        expect(todoMut.todos[0].content).toBe("do thing");
      }
    });

    it("emits canvasAutoOpen for canvas tool", () => {
      const spec = { title: "Test", fields: [] };
      const tc: ToolCallDisplay = { id: "tc1", name: "canvas", arguments: JSON.stringify({ spec }), status: "running" };
      const state = makeState({ isStreaming: true, activeToolCalls: [tc] });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallDone",
        sessionId: "s1",
        toolCallId: "tc1",
        toolName: "canvas",
        output: "ok",
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      const canvasMut = mutations.find((m) => m.type === "canvasAutoOpen");
      expect(canvasMut).toBeDefined();
      if (canvasMut?.type === "canvasAutoOpen") {
        expect(canvasMut.toolCallId).toBe("tc1");
        expect(canvasMut.spec).toEqual(spec);
      }
    });
  });

  describe("toolCallDelta", () => {
    it("appends delta to tool call", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "toolCallDelta", sessionId: "s1", toolCallId: "tc1", delta: "partial" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "appendToolDelta", id: "tc1", delta: "partial" });
    });
  });

  describe("toolCallProgress", () => {
    it("updates structured tool progress without appending output", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = {
        runId: "test-run",
        type: "toolCallProgress",
        sessionId: "s1",
        toolCallId: "tc1",
        title: "Compiling states",
        info: "",
        progress: null,
        state: "running",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateToolProgress",
        id: "tc1",
        progress: {
          title: "Compiling states",
          info: "",
          progress: null,
          state: "running",
        },
      });
      expect(mutations.some((mutation) => mutation.type === "appendToolDelta")).toBe(false);
    });
  });

  describe("knowledgeProposal", () => {
    it("upserts proposal messages into the stream", () => {
      const event: StreamEvent = {
        runId: "test-run",
        type: "knowledgeProposal",
        sessionId: "s1",
        message: {
          id: "kp-msg-1",
          role: "assistant",
          content: "",
          createdAt: 1,
          knowledgeProposal: {
            proposalId: "kp-1",
            status: "pending",
            confidence: 0.82,
            verify: "required",
            estTokens: 1200,
            items: [
              {
                kind: "memory",
                mode: "replace",
                target: "project-understanding.md",
                draft: "# Project Understanding",
              },
            ],
            createdAt: 1,
            updatedAt: 1,
          },
        },
      };

      const mutations = reduceStreamEvent(makeState(), event);
      expect(mutations).toContainEqual({
        type: "upsertMessage",
        message: event.message,
      });
    });
  });

  describe("userMessage", () => {
    it("emits a dedicated mutation for persisted user messages", () => {
      const event: StreamEvent = {
        runId: "test-run",
        type: "userMessage",
        sessionId: "s1",
        message: {
          id: "user-1",
          role: "user",
          content: "hello",
          createdAt: 10,
        },
      };

      expect(reduceStreamEvent(makeState(), event)).toContainEqual({
        type: "upsertUserMessage",
        message: event.message,
      });
    });

    it("replaces the optimistic pending user message with the persisted message", () => {
      const messages = mergeUserMessage([
        {
          id: "user_pending_1",
          role: "user",
          content: "hello",
          createdAt: 10,
        },
      ], {
        id: "user-1",
        role: "user",
        content: "hello",
        createdAt: 10,
      });

      expect(messages).toEqual([
        {
          id: "user-1",
          role: "user",
          content: "hello",
          createdAt: 10,
        },
      ]);
    });

    it("matches pending user messages by client message id when persisted content differs", () => {
      const signature = JSON.stringify({
        kind: "user_intent_v1",
        mode: "build",
        skills: [],
        clientMessageId: "user_pending_1",
      });
      const messages = mergeUserMessage([
        {
          id: "user_pending_1",
          role: "user",
          content: "inspect this asset",
          createdAt: 10,
          thinkingSignature: signature,
        },
      ], {
        id: "user-1",
        role: "user",
        content: "inspect this asset\n\n<locus-references>\n- asset: {@Assets/Foo.prefab}\n</locus-references>",
        createdAt: 11,
        thinkingSignature: signature,
      });

      expect(messages).toHaveLength(1);
      expect(messages[0]?.id).toBe("user-1");
      expect(messages[0]?.content).toContain("<locus-references>");
    });

    it("does not replace a pending user message with different content only because timestamps are close", () => {
      const messages = mergeUserMessage([
        {
          id: "user_pending_1",
          role: "user",
          content: "new message",
          createdAt: 10,
        },
      ], {
        id: "user-1",
        role: "user",
        content: "old message",
        createdAt: 11,
      });

      expect(messages).toEqual([
        {
          id: "user_pending_1",
          role: "user",
          content: "new message",
          createdAt: 10,
        },
        {
          id: "user-1",
          role: "user",
          content: "old message",
          createdAt: 11,
        },
      ]);
    });

    it("keeps pending messages separate when client message ids differ", () => {
      const pendingSignature = JSON.stringify({
        kind: "user_intent_v1",
        mode: "build",
        skills: [],
        clientMessageId: "user_pending_new",
      });
      const persistedSignature = JSON.stringify({
        kind: "user_intent_v1",
        mode: "build",
        skills: [],
        clientMessageId: "user_pending_old",
      });
      const messages = mergeUserMessage([
        {
          id: "user_pending_new",
          role: "user",
          content: "same text",
          createdAt: 10,
          thinkingSignature: pendingSignature,
        },
      ], {
        id: "user-old",
        role: "user",
        content: "same text",
        createdAt: 11,
        thinkingSignature: persistedSignature,
      });

      expect(messages.map((message) => message.id)).toEqual(["user_pending_new", "user-old"]);
    });
  });

  describe("subagent tool calls", () => {
    it("adds nested tool call under parent", () => {
      const parent: ToolCallDisplay = { id: "p1", name: "agent", arguments: "{}", status: "running", nestedToolCalls: [] };
      const state = makeState({ isStreaming: true, activeToolCalls: [parent] });
      const event: StreamEvent = { runId: "test-run",
        type: "subagentToolCallStart",
        sessionId: "s1",
        parentToolCallId: "p1",
        toolCallId: "c1",
        toolName: "read",
        arguments: "{}",
      };
      const mutations = reduceStreamEvent(state, event);

      const addMut = mutations.find((m) => m.type === "addNestedToolCall");
      expect(addMut).toBeDefined();
      if (addMut?.type === "addNestedToolCall") {
        expect(addMut.parentId).toBe("p1");
        expect(addMut.toolCall.id).toBe("c1");
      }
    });

    it("marks nested tool call done", () => {
      const child: ToolCallDisplay = { id: "c1", name: "read", arguments: "{}", status: "running" };
      const parent: ToolCallDisplay = { id: "p1", name: "agent", arguments: "{}", status: "running", nestedToolCalls: [child] };
      const state = makeState({ isStreaming: true, activeToolCalls: [parent] });
      const event: StreamEvent = { runId: "test-run",
        type: "subagentToolCallDone",
        sessionId: "s1",
        parentToolCallId: "p1",
        toolCallId: "c1",
        toolName: "read",
        output: "ok",
        outcome: "done",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "updateNestedToolCall",
        parentId: "p1",
        childId: "c1",
        updates: { status: "done", output: "ok" },
      });
    });
  });

  describe("toolCallRoundDone", () => {
    it("pushes assistant message with render parts, tool results, and clears live round", () => {
      const state = makeState({
        isStreaming: true,
        streamingTextOrder: 3,
        streamingThinking: "thought",
        thinkingOrder: 1,
        thinkingDuration: 3,
      });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallRoundDone",
        sessionId: "s1",
        messageId: "m1",
        fullText: "result text",
        toolCalls: [{ id: "tc1", name: "read", arguments: "{}" }],
        renderParts: [
          {
            kind: "thinking",
            id: "think-1",
            order: { runId: "test-run", seq: 1 },
            content: "thought",
          },
          {
            kind: "toolCall",
            id: "tc1",
            order: { runId: "test-run", seq: 2 },
            toolCall: { id: "tc1", name: "read", arguments: "{}" },
          },
          {
            kind: "text",
            id: "text-1",
            order: { runId: "test-run", seq: 3 },
            content: "result text",
          },
        ],
      };
      const mutations = reduceStreamEvent(state, event);

      const pushMsg = mutations.find((m) => m.type === "pushMessage");
      expect(pushMsg).toBeDefined();
      if (pushMsg?.type === "pushMessage") {
        expect(pushMsg.message.id).toBe("m1");
        expect(pushMsg.message.role).toBe("assistant");
        expect(pushMsg.message.content).toBe("result text");
        expect(pushMsg.message.thinkingContent).toBe("thought");
        expect(pushMsg.message.thinkingDuration).toBe(3);
        expect(pushMsg.message.contentOrder).toBe(3);
        expect(pushMsg.message.thinkingOrder).toBe(1);
        expect(pushMsg.message.renderParts?.map((part) => part.kind)).toEqual(["thinking", "toolCall", "text"]);
      }
      expect(mutations).toContainEqual({ type: "pushToolResults", toolCallIds: ["tc1"] });
      expect(mutations.find((m) => m.type === "clearLiveRenderParts")).toBeDefined();
      expect(mutations.find((m) => m.type === "resetRound")).toBeDefined();
      expect(mutations.find((m) => m.type === "resetRoundKeepToolCalls")).toBeUndefined();
    });

    it("keeps streamed render order when toolCallRoundDone carries round-local order", () => {
      const state = makeState({ isStreaming: true, streamingTextOrder: 3, streamingThinking: "thought", thinkingOrder: 1 });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallRoundDone",
        sessionId: "s1",
        messageId: "m1",
        fullText: "result text",
        toolCalls: [{ id: "tc1", name: "read", arguments: "{}", order: 2 }],
        contentOrder: 8,
        thinkingOrder: 7,
      };
      const mutations = reduceStreamEvent(state, event);
      const pushMsg = mutations.find((m) => m.type === "pushMessage");

      expect(pushMsg?.type === "pushMessage" ? pushMsg.message.contentOrder : 0).toBe(3);
      expect(pushMsg?.type === "pushMessage" ? pushMsg.message.thinkingOrder : 0).toBe(1);
      expect(pushMsg?.type === "pushMessage" ? pushMsg.message.toolCalls?.[0]?.order : 0).toBe(2);
    });

    it("normalizes final-only message order while preserving thinking/content order", () => {
      const state = makeState({ isStreaming: true, streamSequence: 10, streamingThinking: "thought" });
      const event: StreamEvent = { runId: "test-run",
        type: "toolCallRoundDone",
        sessionId: "s1",
        messageId: "m1",
        fullText: "result text",
        toolCalls: [{ id: "tc1", name: "read", arguments: "{}", order: 9 }],
        contentOrder: 2,
        thinkingOrder: 1,
      };
      const mutations = reduceStreamEvent(state, event);
      const pushMsg = mutations.find((m) => m.type === "pushMessage");

      expect(pushMsg?.type === "pushMessage" ? pushMsg.message.thinkingOrder : 0).toBe(11);
      expect(pushMsg?.type === "pushMessage" ? pushMsg.message.contentOrder : 0).toBe(12);
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 11 });
      expect(mutations).toContainEqual({ type: "setStreamSequence", value: 12 });
    });
  });

  describe("usageUpdate", () => {
    it("updates all usage fields", () => {
      const state = makeState();
      const event: StreamEvent = { runId: "test-run",
        type: "usageUpdate",
        sessionId: "s1",
        inputTokens: 100,
        outputTokens: 50,
        cacheReadTokens: 10,
        cacheWriteTokens: 5,
        totalInputTokens: 200,
        totalOutputTokens: 100,
        totalCacheReadTokens: 20,
        totalCacheWriteTokens: 10,
        totalCostUsd: 0.05,
        pricedRounds: 3,
        contextTokens: 5000,
        contextLimit: 100000,
      };
      const mutations = reduceStreamEvent(state, event);

      const usageMut = mutations.find((m) => m.type === "updateUsage");
      expect(usageMut).toBeDefined();
      if (usageMut?.type === "updateUsage") {
        expect(usageMut.usage.totalInputTokens).toBe(200);
        expect(usageMut.usage.totalCostUsd).toBe(0.05);
        expect(usageMut.usage.contextTokens).toBe(5000);
      }
    });

    it("preserves existing contextTokens when event has 0", () => {
      const state = makeState({
        tokenUsage: { totalInputTokens: 0, totalOutputTokens: 0, totalCacheReadTokens: 0, totalCacheWriteTokens: 0, totalCostUsd: 0, pricedRounds: 0, contextTokens: 3000, contextLimit: 100000 },
      });
      const event: StreamEvent = { runId: "test-run",
        type: "usageUpdate",
        sessionId: "s1",
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        totalInputTokens: 100,
        totalOutputTokens: 50,
        totalCacheReadTokens: 0,
        totalCacheWriteTokens: 0,
        totalCostUsd: 0.01,
        pricedRounds: 1,
        contextTokens: 0,
        contextLimit: 0,
      };
      const mutations = reduceStreamEvent(state, event);

      const usageMut = mutations.find((m) => m.type === "updateUsage");
      if (usageMut?.type === "updateUsage") {
        expect(usageMut.usage.contextTokens).toBe(3000);
        expect(usageMut.usage.contextLimit).toBe(100000);
      }
    });
  });

  describe("compactStart", () => {
    it("marks context compaction as visible and preserves token totals", () => {
      const state = makeState({
        tokenUsage: {
          totalInputTokens: 100,
          totalOutputTokens: 50,
          totalCacheReadTokens: 10,
          totalCacheWriteTokens: 5,
          totalCostUsd: 0.01,
          pricedRounds: 1,
          contextTokens: 0,
          contextLimit: 0,
        },
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "compactStart",
        sessionId: "s1",
        contextTokens: 90000,
        contextLimit: 100000,
      };

      const mutations = reduceStreamEvent(state, event);
      expect(mutations).toContainEqual({ type: "setCompacting", value: true });
      const usageMut = mutations.find((m) => m.type === "updateUsage");
      expect(usageMut).toBeDefined();
      if (usageMut?.type === "updateUsage") {
        expect(usageMut.usage.contextTokens).toBe(90000);
        expect(usageMut.usage.contextLimit).toBe(100000);
        expect(usageMut.usage.totalInputTokens).toBe(100);
      }
    });
  });

  describe("compactDone", () => {
    it("updates context usage with the compacted prompt estimate", () => {
      const state = makeState({
        tokenUsage: {
          totalInputTokens: 100,
          totalOutputTokens: 50,
          totalCacheReadTokens: 10,
          totalCacheWriteTokens: 5,
          totalCostUsd: 0.01,
          pricedRounds: 1,
          contextTokens: 8000,
          contextLimit: 100000,
        },
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "compactDone",
        sessionId: "s1",
        messagesBefore: 40,
        messagesAfter: 8,
        contextTokens: 2400,
        contextLimit: 100000,
        messages: [
          {
            id: "user-1",
            role: "user",
            content: "older visible request",
            createdAt: 90,
          },
          {
            id: "assistant-1",
            role: "assistant",
            content: "older visible answer",
            createdAt: 100,
          },
        ],
      };

      const mutations = reduceStreamEvent(state, event);
      const replaceMut = mutations.find((m) => m.type === "replaceMessages");
      expect(replaceMut).toBeDefined();
      if (replaceMut?.type === "replaceMessages") {
        expect(replaceMut.messages).toHaveLength(2);
        expect(replaceMut.messages.map((message) => message.id)).toEqual(["user-1", "assistant-1"]);
      }
      const usageMut = mutations.find((m) => m.type === "updateUsage");
      expect(usageMut).toBeDefined();
      if (usageMut?.type === "updateUsage") {
        expect(usageMut.usage.contextTokens).toBe(2400);
        expect(usageMut.usage.contextLimit).toBe(100000);
        expect(usageMut.usage.totalInputTokens).toBe(100);
      }
      expect(mutations).toContainEqual({ type: "setCompacting", value: false });
    });

    it("keeps previous context usage for compactDone events recorded before context fields existed", () => {
      const state = makeState({
        tokenUsage: {
          totalInputTokens: 100,
          totalOutputTokens: 50,
          totalCacheReadTokens: 10,
          totalCacheWriteTokens: 5,
          totalCostUsd: 0.01,
          pricedRounds: 1,
          contextTokens: 8000,
          contextLimit: 100000,
        },
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "compactDone",
        sessionId: "s1",
        messagesBefore: 40,
        messagesAfter: 8,
        messages: [],
      };

      const mutations = reduceStreamEvent(state, event);
      expect(mutations.some((m) => m.type === "updateUsage")).toBe(false);
      expect(mutations).toContainEqual({ type: "setCompacting", value: false });
    });
  });

  describe("askUser", () => {
    it("sets pending question", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "askUser",
        sessionId: "s1",
        questionId: "q1",
        toolCallId: "tc1",
        question: "What file?",
        options: [{ label: "foo", description: "foo.ts" }],
      };
      const mutations = reduceStreamEvent(state, event);

      const qMut = mutations.find((m) => m.type === "setQuestion");
      expect(qMut).toBeDefined();
      if (qMut?.type === "setQuestion") {
        expect(qMut.question?.questionId).toBe("q1");
        expect(qMut.question?.question).toBe("What file?");
      }
    });
  });

  describe("toolConfirm", () => {
    it("sets pending tool confirm", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "toolConfirm",
        sessionId: "s1",
        questionId: "q1",
        toolCallId: "tc1",
        display: {
          kind: "knowledge",
          operation: "edit",
          targetKind: "document",
          docType: "design",
          path: "design/core.md",
          directoryPath: "design",
          directoryMode: "approval",
          documentBeforeText: "before",
          documentAfterText: "after",
        },
      };
      const mutations = reduceStreamEvent(state, event);

      const cMut = mutations.find((m) => m.type === "enqueueToolConfirm");
      expect(cMut).toBeDefined();
      if (cMut?.type === "enqueueToolConfirm") {
        expect(cMut.confirm?.display.kind).toBe("knowledge");
      }
    });
  });

  describe("inputAnswered", () => {
    it("clears the matching pending input by question id", () => {
      const state = makeState({
        pendingQuestion: {
          questionId: "q1",
          toolCallId: "ask-1",
          question: "Continue?",
          options: [],
        },
        pendingToolConfirms: [
          {
            questionId: "q2",
            toolCallId: "tc1",
            display: {
              kind: "basic",
              toolName: "write",
              arguments: "{}",
            },
          },
        ],
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "inputAnswered",
        sessionId: "s1",
        questionId: "q2",
      };

      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({
        type: "clearPendingInput",
        questionId: "q2",
      });
    });
  });

  describe("undoAvailable", () => {
    it("adds message id to undoable set", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run", type: "undoAvailable", sessionId: "s1", assistantMessageId: "m1" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "addUndoable", messageId: "m1" });
    });
  });

  describe("done", () => {
    it("upserts final message, resets round, and stops streaming", () => {
      const state = makeState({ isStreaming: true, streamingThinking: "", thinkingDuration: 0 });
      const event: StreamEvent = { runId: "test-run", type: "done", sessionId: "s1", messageId: "m1", fullText: "final text" };
      const mutations = reduceStreamEvent(state, event);

      const upsertMsg = mutations.find((m) => m.type === "upsertMessage");
      expect(upsertMsg).toBeDefined();
      if (upsertMsg?.type === "upsertMessage") {
        expect(upsertMsg.message.content).toBe("final text");
        expect(upsertMsg.message.thinkingContent).toBeUndefined();
      }
      expect(mutations).toContainEqual({ type: "resetRound" });
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });

    it("stops streaming without pushing an empty assistant message", () => {
      const state = makeState({ isStreaming: true, streamingThinking: "thought", thinkingDuration: 2 });
      const event: StreamEvent = { runId: "test-run", type: "done", sessionId: "s1", messageId: "", fullText: "" };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.find((m) => m.type === "upsertMessage")).toBeUndefined();
      expect(mutations).toContainEqual({ type: "resetRound" });
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });

    it("reuses the same assistant message id when a server-tool round already inserted the message", () => {
      const state = makeState({
        isStreaming: true,
        messages: [
          {
            id: "m1",
            role: "assistant",
            content: "final text",
            createdAt: 1,
            toolCalls: [{ id: "ws-1", name: "web_search", arguments: "{\"query\":\"unity\"}", serverToolOutput: "Searched: unity" }],
          },
        ],
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "done",
        sessionId: "s1",
        messageId: "m1",
        fullText: "final text",
      };
      const mutations = reduceStreamEvent(state, event);

      const upsertMsg = mutations.find((m) => m.type === "upsertMessage");
      expect(upsertMsg).toBeDefined();
      if (upsertMsg?.type === "upsertMessage") {
        expect(upsertMsg.message.id).toBe("m1");
        expect(upsertMsg.message.content).toBe("final text");
        expect(upsertMsg.message.toolCalls).toEqual([
          {
            id: "ws-1",
            name: "web_search",
            arguments: "{\"query\":\"unity\"}",
            serverToolOutput: "Searched: unity",
          },
        ]);
      }
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });
  });

  describe("error", () => {
    it("stops streaming without producing inline error state", () => {
      const state = makeState({ isStreaming: true });
      const event: StreamEvent = { runId: "test-run",
        type: "error",
        sessionId: "s1",
        error: { code: "test.error", message: "something broke", retryable: false, severity: "error" },
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations.find((m) => m.type === "pushMessage")).toBeUndefined();
      expect(mutations.find((m) => m.type === "resetRound")).toBeDefined();
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });
  });

  describe("cancelled", () => {
    it("stops streaming silently when the run is cancelled", () => {
      const state = makeState({ isStreaming: true, streamingThinking: "thinking" });
      const event: StreamEvent = {
        runId: "test-run",
        type: "cancelled",
        sessionId: "s1",
      };
      const mutations = reduceStreamEvent(state, event);

      expect(mutations).toContainEqual({ type: "resetRound" });
      expect(mutations).toContainEqual({ type: "clearPendingInputs" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
      expect(mutations.find((m) => m.type === "pushMessage")).toBeUndefined();
    });

    it("keeps interrupted assistant text when cancellation persists a message", () => {
      const state = makeState({
        isStreaming: true,
        rawStreamText: "partial answer",
        streamingThinking: "partial thought",
        thinkingDuration: 2,
      });
      const event: StreamEvent = {
        runId: "test-run",
        type: "cancelled",
        sessionId: "s1",
        messageId: "m-cancelled",
        fullText: "partial answer",
        thinkingContent: "partial thought",
        thinkingDuration: 2,
      };
      const mutations = reduceStreamEvent(state, event);

      const upsert = mutations.find((m) => m.type === "upsertMessage");
      expect(upsert).toBeDefined();
      if (upsert?.type === "upsertMessage") {
        expect(upsert.message.id).toBe("m-cancelled");
        expect(upsert.message.role).toBe("assistant");
        expect(upsert.message.content).toBe("partial answer");
        expect(upsert.message.thinkingContent).toBe("partial thought");
        expect(upsert.message.thinkingDuration).toBe(2);
      }
      expect(mutations).toContainEqual({ type: "resetRound" });
      expect(mutations).toContainEqual({ type: "setStreaming", value: false });
    });
  });
});

````

### src/types.ts:1-240

````ts
    1: export type SessionRuntimeStatus =
    2:   | "running"
    3:   | "queued"
    4:   | "starting"
    5:   | "waiting_input"
    6:   | "cancelling"
    7:   | "error";
    8: 
    9: export interface SessionSummary {
   10:   id: string;
   11:   title: string;
   12:   agentId?: string | null;
   13:   sessionType: string;
   14:   parentSessionId?: string | null;
   15:   updatedAt: number;
   16:   runtimeStatus?: SessionRuntimeStatus | null;
   17: }
   18: 
   19: export type ServerToolKind = "web_search";
   20: 
   21: export interface ToolCallInfo {
   22:   id: string;
   23:   name: string;
   24:   arguments: string;
   25:   order?: number;
   26:   serverTool?: ServerToolKind;
   27:   serverToolOutput?: string;
   28:   outcome?: ToolCallOutcome;
   29:   recordedOutput?: string;
   30:   nestedToolCalls?: ToolCallInfo[];
   31: }
   32: 
   33: export type ToolCallOutcome = "done" | "error" | "interrupted";
   34: 
   35: export interface RenderOrderKey {
   36:   runId: string;
   37:   seq: number;
   38: }
   39: 
   40: export type AssistantRenderPart =
   41:   | {
   42:       kind: "thinking";
   43:       id: string;
   44:       order: RenderOrderKey;
   45:       content: string;
   46:       active?: boolean;
   47:       duration?: number;
   48:       signature?: string;
   49:     }
   50:   | {
   51:       kind: "text";
   52:       id: string;
   53:       order: RenderOrderKey;
   54:       content: string;
   55:     }
   56:   | {
   57:       kind: "toolCall";
   58:       id: string;
   59:       order: RenderOrderKey;
   60:       toolCall: ToolCallInfo;
   61:     }
   62:   | {
   63:       kind: "knowledgeProposal";
   64:       id: string;
   65:       order: RenderOrderKey;
   66:       message: ChatMessage;
   67:     };
   68: 
   69: export interface ImageAttachment {
   70:   data: string;
   71:   mimeType: string;
   72: }
   73: 
   74: export type AssetRefKind = "asset" | "sceneObject" | "knowledge";
   75: 
   76: export interface AssetRefAttachment {
   77:   path: string;
   78:   kind: AssetRefKind;
   79:   name?: string;
   80:   typeLabel?: string;
   81:   source?: "unity" | "manual";
   82: }
   83: 
   84: export interface UnityConnectionStatus {
   85:   connected: boolean;
   86:   editorStatus: string;
   87:   scenePath?: string | null;
   88:   pipeName: string;
   89:   latencyMs?: number | null;
   90:   reconnectAttempts: number;
   91:   lastError?: string | null;
   92:   checkedAtMs: number;
   93: }
   94: 
   95: export interface SkillIntentItem {
   96:   dirName: string;
   97:   source: "app" | "project";
   98:   name: string;
   99: }
  100: 
  101: export interface UserIntentMeta {
  102:   kind: "user_intent_v1";
  103:   mode: "build" | "plan";
  104:   skills: SkillIntentItem[];
  105:   clientMessageId?: string;
  106: }
  107: 
  108: export interface ChatComposerSendPayload {
  109:   text: string;
  110:   displayText: string;
  111:   images: ImageAttachment[];
  112:   assetRefs: AssetRefAttachment[];
  113:   mode?: string | null;
  114:   userIntent?: UserIntentMeta | null;
  115: }
  116: 
  117: export type KnowledgeProposalVerify = "none" | "required";
  118: export type KnowledgeProposalStatus =
  119:   | "pending"
  120:   | "applying"
  121:   | "applied"
  122:   | "invalidated"
  123:   | "stale";
  124: export type KnowledgeProposalItemKind = "memory" | "knowledge";
  125: export type KnowledgeProposalItemMode =
  126:   | "replace"
  127:   | "create_source"
  128:   | "update_source";
  129: 
  130: export interface KnowledgeProposalItem {
  131:   kind: KnowledgeProposalItemKind;
  132:   mode: KnowledgeProposalItemMode;
  133:   target: string;
  134:   draft: string;
  135: }
  136: 
  137: export interface KnowledgeProposal {
  138:   proposalId: string;
  139:   status: KnowledgeProposalStatus;
  140:   confidence: number;
  141:   verify: KnowledgeProposalVerify;
  142:   estTokens: number;
  143:   items: KnowledgeProposalItem[];
  144:   createdAt: number;
  145:   updatedAt: number;
  146: }
  147: 
  148: export interface ChatMessage {
  149:   id: string;
  150:   role: "user" | "assistant" | "tool";
  151:   content: string;
  152:   createdAt: number;
  153:   promptPrefix?: string;
  154:   promptSuffix?: string;
  155:   responseId?: string;
  156:   contentOrder?: number;
  157:   thinkingOrder?: number;
  158:   toolCalls?: ToolCallInfo[];
  159:   toolCallId?: string;
  160:   images?: ImageAttachment[];
  161:   assetRefs?: AssetRefAttachment[];
  162:   thinkingContent?: string;
  163:   thinkingDuration?: number;
  164:   thinkingSignature?: string;
  165:   intentMeta?: UserIntentMeta;
  166:   knowledgeProposal?: KnowledgeProposal;
  167:   renderParts?: AssistantRenderPart[];
  168: }
  169: 
  170: export interface SessionDetail {
  171:   id: string;
  172:   title: string;
  173:   agentId?: string | null;
  174:   sessionType: string;
  175:   parentSessionId: string | null;
  176:   latestCompletedRunId?: string | null;
  177:   createdAt: number;
  178:   updatedAt: number;
  179:   messages: ChatMessage[];
  180: }
  181: 
  182: export type SessionRunStatus =
  183:   | "queued"
  184:   | "starting"
  185:   | "running"
  186:   | "waiting_input"
  187:   | "cancelling"
  188:   | "done"
  189:   | "cancelled"
  190:   | "error";
  191: 
  192: export interface SessionRunSummary {
  193:   runId: string;
  194:   sessionId: string;
  195:   status: SessionRunStatus;
  196:   startedAt: number;
  197:   updatedAt: number;
  198:   finishedAt?: number | null;
  199:   errorMessage?: string | null;
  200: }
  201: 
  202: export interface SessionEventRecord {
  203:   sessionId: string;
  204:   runId: string;
  205:   seq: number;
  206:   eventType: string;
  207:   payload: Record<string, unknown>;
  208:   createdAt: number;
  209: }
  210: 
  211: export interface ActiveSessionSelectionChanged {
  212:   workspaceKey: string;
  213:   sessionId: string | null;
  214: }
  215: 
  216: export interface SaveRawContextRequest {
  217:   sessionId: string;
  218:   includeSystemPrompt: boolean;
  219: }
  220: 
  221: export interface AgentInfo {
  222:   id: string;
  223:   name: string;
  224:   description: string;
  225:   isDefault: boolean;
  226:   defaultEffort?: EffortLevel | null;
  227:   modelRecommendation?: ModelRecommendation | null;
  228:   source: string;
  229: }
  230: 
  231: export type EffortLevel = "none" | "low" | "medium" | "high" | "xhigh" | "max";
  232: export type ThinkingLevel = EffortLevel;
  233: export type ModelRecommendation = "small" | "large";
  234: 
  235: export interface ModelOption {
  236:   id: string;
  237:   name: string;
  238:   provider:
  239:     | "openrouter"
  240:     | "anthropic"
````

### src/types.ts:240-520

````ts
  240:     | "anthropic"
  241:     | "anthropic_sdk"
  242:     | "openai_codex"
  243:     | "custom";
  244:   defaultEffort?: EffortLevel | null;
  245:   supportedEfforts?: EffortLevel[];
  246:   additionalSpeedTiers?: string[];
  247:   isDefault?: boolean;
  248: }
  249: 
  250: export type ApiFormat =
  251:   | "openai_chat"
  252:   | "openai_responses"
  253:   | "anthropic_messages";
  254: 
  255: export type ReasoningParamFormat =
  256:   | "none"
  257:   | "openai_chat_reasoning_effort"
  258:   | "openai_responses_reasoning_effort"
  259:   | "anthropic_thinking";
  260: 
  261: export interface CustomEndpointServerTools {
  262:   webSearch: boolean;
  263: }
  264: 
  265: export interface CustomEndpoint {
  266:   id: string;
  267:   name: string;
  268:   apiModel: string;
  269:   endpoint: string;
  270:   apiFormat: ApiFormat;
  271:   apiKey: string;
  272:   contextLength: number;
  273:   betaFlags: string[];
  274:   supportedReasoningEfforts: EffortLevel[];
  275:   reasoningParamFormat: ReasoningParamFormat;
  276:   replayReasoningContent: boolean;
  277:   serverTools: CustomEndpointServerTools;
  278: }
  279: 
  280: export interface ModelDefaults {
  281:   mainModel: string;
  282:   planModel: string;
  283:   subagentModels: Record<string, string>;
  284: }
  285: 
  286: export type CodexTransportMode = "http" | "websocket";
  287: 
  288: export interface CodexModelConfig {
  289:   transport: CodexTransportMode;
  290: }
  291: 
  292: export interface AuthStatus {
  293:   authenticated: boolean;
  294:   hasApiKey: boolean;
  295:   email: string | null;
  296: }
  297: 
  298: export interface AppStorageInfo {
  299:   activePath: string;
  300:   defaultPath: string;
  301:   activeSizeBytes: number;
  302:   usesCustomPath: boolean;
  303:   pendingTargetPath?: string | null;
  304:   restartRequired: boolean;
  305: }
  306: 
  307: export type PythonRuntimeSource = "managed" | "system";
  308: 
  309: export interface PythonRuntimeInfo {
  310:   id: string;
  311:   label: string;
  312:   path: string;
  313:   version?: string | null;
  314:   source: PythonRuntimeSource;
  315:   selected: boolean;
  316:   available: boolean;
  317: }
  318: 
  319: export interface PythonRuntimeState {
  320:   runtimes: PythonRuntimeInfo[];
  321:   selectedId?: string | null;
  322:   effective?: PythonRuntimeInfo | null;
  323:   missingSelected: boolean;
  324: }
  325: 
  326: export type GitRuntimeSource = "envOverride" | "managed" | "path" | "commonLocation";
  327: 
  328: export interface GitRuntimeInfo {
  329:   id: string;
  330:   label: string;
  331:   path: string;
  332:   version?: string | null;
  333:   source: GitRuntimeSource;
  334:   selected: boolean;
  335:   available: boolean;
  336: }
  337: 
  338: export interface GitRuntimeState {
  339:   runtimes: GitRuntimeInfo[];
  340:   selectedId?: string | null;
  341:   effective?: GitRuntimeInfo | null;
  342:   missingSelected: boolean;
  343: }
  344: 
  345: export type ProxyMode = "auto" | "manual" | "disabled";
  346: export type ProxyEnvironmentEntryKind = "proxy" | "bypass";
  347: export type ProxyRouteSource = "system" | "environment" | "manual" | "direct";
  348: 
  349: export interface ProxyEnvironmentEntry {
  350:   key: string;
  351:   value: string;
  352:   kind: ProxyEnvironmentEntryKind;
  353: }
  354: 
  355: export interface SystemProxyConfig {
  356:   platform: string;
  357:   available: boolean;
  358:   source: string;
  359:   enabled?: boolean | null;
  360:   autoDetect?: boolean | null;
  361:   autoConfigUrl?: string | null;
  362:   proxyServer?: string | null;
  363:   proxyOverride?: string | null;
  364:   httpProxy?: string | null;
  365:   httpsProxy?: string | null;
  366:   socksProxy?: string | null;
  367: }
  368: 
  369: export interface ManualProxyConfig {
  370:   httpProxy: string;
  371:   httpsProxy: string;
  372:   allProxy: string;
  373:   noProxy: string;
  374: }
  375: 
  376: export interface ProxyConfig {
  377:   mode: ProxyMode;
  378:   manual: ManualProxyConfig;
  379: }
  380: 
  381: export interface ProxyRoute {
  382:   targetLabel: string;
  383:   targetUrl: string;
  384:   proxyUrl?: string | null;
  385:   source: ProxyRouteSource;
  386: }
  387: 
  388: export interface ProxyStatus {
  389:   mode: ProxyMode;
  390:   config: ProxyConfig;
  391:   environment: ProxyEnvironmentEntry[];
  392:   /** Backward-compatible alias for older backend status payloads. Prefer environment. */
  393:   manual: ProxyEnvironmentEntry[];
  394:   system: SystemProxyConfig;
  395:   routes: ProxyRoute[];
  396: }
  397: 
  398: export interface AuthUrlInfo {
  399:   url: string;
  400: }
  401: 
  402: export interface AppUpdateChangeGroup {
  403:   title: string;
  404:   items: string[];
  405: }
  406: 
  407: export interface AppUpdateDownloadChannel {
  408:   label: string;
  409:   url: string;
  410: }
  411: 
  412: export interface AppUpdateLocaleEntry {
  413:   title: string;
  414:   summary: string;
  415:   changelogUrl: string;
  416:   changes: AppUpdateChangeGroup[];
  417:   downloadChannels?: AppUpdateDownloadChannel[];
  418: }
  419: 
  420: export interface AppUpdateManifest {
  421:   version: string;
  422:   releasedAt: string;
  423:   channel: string;
  424:   locales: Record<string, AppUpdateLocaleEntry>;
  425: }
  426: 
  427: export type AppUpdateSourceKind = "local" | "remote";
  428: 
  429: export interface AppUpdateManifestFetchResult {
  430:   manifest: AppUpdateManifest;
  431:   sourceKind: AppUpdateSourceKind;
  432:   sourceBaseUrl: string;
  433: }
  434: 
  435: export interface AppUpdateInfo {
  436:   currentVersion: string;
  437:   latestVersion: string;
  438:   releasedAt: string;
  439:   channel: string;
  440:   title: string;
  441:   summary: string;
  442:   changelogUrl: string;
  443:   changes: AppUpdateChangeGroup[];
  444:   sourceKind: AppUpdateSourceKind;
  445:   sourceBaseUrl: string;
  446: }
  447: 
  448: export interface TokenUsage {
  449:   totalInputTokens: number;
  450:   totalOutputTokens: number;
  451:   totalCacheReadTokens: number;
  452:   totalCacheWriteTokens: number;
  453:   totalCostUsd: number;
  454:   pricedRounds: number;
  455:   contextTokens: number;
  456:   contextLimit: number;
  457: }
  458: 
  459: // ── Todo ──
  460: 
  461: export interface TodoItem {
  462:   content: string;
  463:   status: "pending" | "in_progress" | "completed" | "cancelled";
  464:   priority: "high" | "medium" | "low";
  465: }
  466: 
  467: export interface TodoSnapshot {
  468:   items: TodoItem[];
  469:   latestRunId: string | null;
  470: }
  471: 
  472: export type TodoPanelMode = "current" | "all";
  473: 
  474: export interface DuplicateGuidOverview {
  475:   groupCount: number;
  476:   pathCount: number;
  477:   assetsOnlyGroups: number;
  478:   packagesOnlyGroups: number;
  479:   crossRootGroups: number;
  480: }
  481: 
  482: export type AssetRiskKind =
  483:   | "brokenReferences"
  484:   | "missingScripts"
  485:   | "parseFailures"
  486:   | "duplicateGuids";
  487: 
  488: export interface AssetRiskEntry {
  489:   kind: AssetRiskKind;
  490:   count: number;
  491: }
  492: 
  493: export interface ScanStats {
  494:   dirsScanned: number;
  495:   metaFilesFound: number;
  496:   yamlAssetsFound: number;
  497:   nodesAdded: number;
  498:   edgesAdded: number;
  499:   nodesUpdated: number;
  500:   nodesDeleted: number;
  501:   parseFailures: number;
  502:   elapsedMs: number;
  503:   duplicateGuids: DuplicateGuidOverview;
  504: }
  505: 
  506: export type AssetDbScanEvent =
  507:   | { phase: "dirScan" }
  508:   | { phase: "metaParse"; total: number; completed: number }
  509:   | { phase: "yamlParse"; total: number; completed: number }
  510:   | { phase: "dbWrite" }
  511:   | { phase: "reconcile"; verifyHashes: boolean }
  512:   | { phase: "reconcileDone" }
  513:   | { phase: "done"; stats: ScanStats }
  514:   | { phase: "error"; error: AppErrorPayload };
  515: 
  516: export interface KnowledgeChangedEvent {
  517:   workingDir: string;
  518:   source: string;
  519:   changedAt: number;
  520:   docType?: "design" | "memory" | "skill" | "reference";
````

### src/stores/chat.ts:1-180

````ts
    1: import { ref, computed } from "vue";
    2: import { defineStore } from "pinia";
    3: import { useModelStore } from "./model";
    4: import { useAgentStore } from "./agent";
    5: import { useNotificationStore } from "./notification";
    6: import { normalizeAppError } from "../services/errors";
    7: import { getToolPermissionMode, saveToolPermissionMode } from "../services/permissions";
    8: import * as sessionService from "../services/session";
    9: import * as undoService from "../services/undo";
   10: import { buildToolResultMessages, mergeUserMessage, reduceStreamEvent, type StreamMutation } from "../composables/useStreamReducer";
   11: import { hydrateChatMessagesIntent, withClientMessageId } from "../composables/chatInputIntents";
   12: import type { SessionScrollState } from "../composables/chatScrollState";
   13: import { t } from "../i18n";
   14: import { useChatChangesStore } from "./chatChanges";
   15: import { useDisplaySettings } from "../composables/useDisplaySettings";
   16: import { logToolCollapseTrace, previewTraceText } from "../services/toolCollapseTrace";
   17: import type {
   18:   SessionSummary, SessionDetail, ChatMessage, TokenUsage,
   19:   TodoItem, StreamEvent, ImageAttachment, AssetRefAttachment, ToolCallDisplay,
   20:   PendingQuestion, PendingToolConfirm,
   21:   UserIntentMeta,
   22:   KnowledgeProposalStatus,
   23:   UndoConflictInfo,
   24:   TodoSnapshot,
   25:   TodoPanelMode,
   26:   SessionEventRecord,
   27:   SessionRunSummary,
   28:   AssistantRenderPart,
   29: } from "../types";
   30: 
   31: type ToolPermissionMode = "auto" | "ask";
   32: 
   33: function emptyTokenUsage(): TokenUsage {
   34:   return {
   35:     totalInputTokens: 0, totalOutputTokens: 0,
   36:     totalCacheReadTokens: 0, totalCacheWriteTokens: 0,
   37:     totalCostUsd: 0, pricedRounds: 0, contextTokens: 0, contextLimit: 0,
   38:   };
   39: }
   40: 
   41: function hydrateMessages(messages: ChatMessage[]): ChatMessage[] {
   42:   return hydrateChatMessagesIntent(messages);
   43: }
   44: 
   45: function replaceMessageById(list: ChatMessage[], message: ChatMessage): ChatMessage[] {
   46:   const index = list.findIndex((item) => item.id === message.id);
   47:   if (index < 0) return [...list, message];
   48:   const next = [...list];
   49:   next.splice(index, 1, message);
   50:   return next;
   51: }
   52: 
   53: function isActiveRuntimeStatus(status: SessionSummary["runtimeStatus"]): boolean {
   54:   return status === "running"
   55:     || status === "waiting_input"
   56:     || status === "cancelling"
   57:     || status === "starting"
   58:     || status === "queued";
   59: }
   60: 
   61: function isActiveRunStatus(status: SessionRunSummary["status"] | null | undefined): boolean {
   62:   return status === "running"
   63:     || status === "waiting_input"
   64:     || status === "cancelling"
   65:     || status === "starting"
   66:     || status === "queued";
   67: }
   68: 
   69: function streamEventFromRecord(record: SessionEventRecord): StreamEvent | null {
   70:   if (!record.payload || typeof record.payload !== "object") return null;
   71:   const payload = record.payload as Record<string, unknown>;
   72:   if (typeof payload.type !== "string") return null;
   73:   return {
   74:     ...payload,
   75:     runId: record.runId,
   76:   } as StreamEvent;
   77: }
   78: 
   79: function activeReplayEvents(
   80:   records: SessionEventRecord[],
   81:   afterSeq: number,
   82: ): SessionEventRecord[] {
   83:   if (afterSeq > 0) return records;
   84:   return records;
   85: }
   86: 
   87: function normalizeToolPermissionMode(mode: string | null | undefined): ToolPermissionMode {
   88:   return mode === "ask" ? "ask" : "auto";
   89: }
   90: 
   91: function logChatStreamDebug(message: string, detail?: Record<string, unknown>) {
   92:   console.info(`[chat-stream] ${message}`, detail ?? {});
   93: }
   94: 
   95: export const useChatStore = defineStore("chat", () => {
   96:   // -- State --
   97:   const sessions = ref<SessionSummary[]>([]);
   98:   const activeSessionId = ref<string | null>(null);
   99:   const activeSessionType = ref<string | null>(null);
  100:   const messages = ref<ChatMessage[]>([]);
  101:   const streamingText = ref("");
  102:   const rawStreamText = ref("");
  103:   const streamingThinking = ref("");
  104:   const streamSequence = ref(0);
  105:   const streamingTextOrder = ref(0);
  106:   const thinkingOrder = ref(0);
  107:   const liveRenderParts = ref<AssistantRenderPart[]>([]);
  108:   const isStreaming = ref(false);
  109:   const isCompacting = ref(false);
  110:   const currentRunId = ref<string | null>(null);
  111:   const isThinking = ref(false);
  112:   const thinkingStartTime = ref(0);
  113:   const thinkingDuration = ref(0);
  114:   const showThinkingPanel = ref(false);
  115:   const thinkingPanelContent = ref("");
  116:   const activeToolCalls = ref<ToolCallDisplay[]>([]);
  117:   const tokenUsage = ref<TokenUsage>(emptyTokenUsage());
  118:   const todos = ref<TodoItem[]>([]);
  119:   const todoWriteVersion = ref(0);
  120:   const showTodoPanel = ref(false);
  121:   const todoPanelVisibility = ref(new Map<string, boolean>());
  122:   const todoMode = ref<TodoPanelMode>("current");
  123:   const sessionLatestTodoRunIds = ref(new Map<string, string | null>());
  124:   const sessionLatestCompletedRunIds = ref(new Map<string, string | null>());
  125:   const pendingQuestion = ref<PendingQuestion | null>(null);
  126:   const pendingToolConfirms = ref<PendingToolConfirm[]>([]);
  127:   const streamingSessionIds = ref(new Set<string>());
  128:   const undoableMessageIds = ref(new Set<string>());
  129:   const sessionRunIds = ref(new Map<string, string>());
  130:   const replayedSessionEventSeqs = new Map<string, number>();
  131:   const sessionScrollStates = ref(new Map<string, SessionScrollState>());
  132:   const sessionAgentId = ref<string | null>(null);
  133:   const toolPermissionMode = ref<ToolPermissionMode>("auto");
  134:   const sessionAgentLocked = computed(() => !!activeSessionId.value && !!sessionAgentId.value);
  135:   const todoRunBoundaryId = computed(() => {
  136:     const sessionId = activeSessionId.value;
  137:     if (!sessionId) return null;
  138:     if (isStreaming.value) return currentRunId.value;
  139:     return sessionLatestCompletedRunIds.value.get(sessionId) ?? null;
  140:   });
  141:   const latestTodoRunId = computed(() => {
  142:     const sessionId = activeSessionId.value;
  143:     if (!sessionId) return null;
  144:     return sessionLatestTodoRunIds.value.get(sessionId) ?? null;
  145:   });
  146:   const currentTodos = computed(() => {
  147:     if (!todoRunBoundaryId.value) return [];
  148:     if (latestTodoRunId.value !== todoRunBoundaryId.value) return [];
  149:     return todos.value;
  150:   });
  151:   const visibleTodos = computed(() => currentTodos.value);
  152:   const hasAnyTodos = computed(() => todos.value.length > 0);
  153:   const visibleTodoCount = computed(() => visibleTodos.value.length);
  154:   const todoCelebrationEnabled = computed(() => (
  155:     !!todoRunBoundaryId.value && latestTodoRunId.value === todoRunBoundaryId.value
  156:   ));
  157:   const todoCelebrationVersion = computed(() => (
  158:     todoCelebrationEnabled.value ? todoWriteVersion.value : 0
  159:   ));
  160: 
  161:   // Plan run tracking (bound to runId + sessionId to avoid stale state)
  162:   const pendingPlanRun = ref<{
  163:     runId: string | null;  // null until first stream event arrives
  164:     sessionId: string;
  165:     agentId: string;
  166:     requestText: string;
  167:   } | null>(null);
  168: 
  169:   let pendingSessionId: string | null = null;
  170:   let pendingMessageSeq = 0;
  171:   let sessionLoadSeq = 0;
  172:   let pendingManagedSessionId: string | null = null;
  173:   let pendingManagedUnboundSession = false;
  174:   // Sessions started from ChatView can be updated incrementally in-memory.
  175:   // Externally driven sessions (Git/docgen) must be reloaded from the store.
  176:   const managedStreamingSessionIds = new Set<string>();
  177:   const closedRunIds = new Map<string, string>();
  178:   const cancelRequestedRunIds = new Map<string, string>();
  179:   let streamReplayDepth = 0;
  180:   let activeSessionSelectionRestoreAttempted = false;
````

### src/stores/chat.ts:680-950

````ts
  680:         runtimeStatus,
  681:         closedRunId,
  682:       });
  683:       return {
  684:         ...session,
  685:         runtimeStatus: null,
  686:       };
  687:     });
  688: 
  689:     return changed ? normalized : nextSessions;
  690:   }
  691: 
  692:   function applyMutation(m: StreamMutation) {
  693:       switch (m.type) {
  694:       case "appendRawText":
  695:         {
  696:           const rawLenBefore = rawStreamText.value.length;
  697:           const streamingLenBefore = streamingText.value.length;
  698:           rawStreamText.value += m.text;
  699:           if (!streamingText.value) streamingText.value = rawStreamText.value.charAt(0);
  700:           logToolCollapseTrace("chat-store", "appendRawText", {
  701:             deltaLen: m.text.length,
  702:             deltaPreview: previewTraceText(m.text, 48),
  703:             rawLenBefore,
  704:             rawLenAfter: rawStreamText.value.length,
  705:             streamingLenBefore,
  706:             streamingLenAfter: streamingText.value.length,
  707:             injectedFirstVisibleChar: streamingLenBefore === 0 && streamingText.value.length > 0,
  708:           });
  709:           startStreamAnim();
  710:         }
  711:         break;
  712:       case "appendThinking":
  713:         streamingThinking.value += m.text;
  714:         break;
  715:       case "setStreamSequence":
  716:         streamSequence.value = Math.max(streamSequence.value, m.value);
  717:         break;
  718:       case "setStreamingTextOrder":
  719:         streamingTextOrder.value = m.order;
  720:         break;
  721:       case "setThinkingOrder":
  722:         thinkingOrder.value = m.order;
  723:         break;
  724:       case "upsertLiveRenderPart": {
  725:         const index = liveRenderParts.value.findIndex((part) => part.id === m.part.id);
  726:         if (index < 0) {
  727:           liveRenderParts.value = [...liveRenderParts.value, m.part];
  728:         } else {
  729:           const next = [...liveRenderParts.value];
  730:           next.splice(index, 1, { ...next[index]!, ...m.part } as AssistantRenderPart);
  731:           liveRenderParts.value = next;
  732:         }
  733:         break;
  734:       }
  735:       case "appendLiveRenderPartContent":
  736:         liveRenderParts.value = liveRenderParts.value.map((part) => {
  737:           if (part.id !== m.partId) return part;
  738:           if (part.kind !== "thinking" && part.kind !== "text") return part;
  739:           return { ...part, content: part.content + m.text };
  740:         });
  741:         break;
  742:       case "deactivateLiveThinkingParts":
  743:         liveRenderParts.value = liveRenderParts.value.map((part) =>
  744:           part.kind === "thinking"
  745:             ? { ...part, active: false, duration: m.duration ?? part.duration }
  746:             : part,
  747:         );
  748:         break;
  749:       case "updateLiveToolPart":
  750:         liveRenderParts.value = liveRenderParts.value.map((part) =>
  751:           part.kind === "toolCall" && part.toolCall.id === m.toolCallId
  752:             ? { ...part, toolCall: { ...part.toolCall, ...m.updates } }
  753:             : part,
  754:         );
  755:         break;
  756:       case "clearLiveRenderParts":
  757:         liveRenderParts.value = [];
  758:         break;
  759:       case "setThinking":
  760:         isThinking.value = m.value;
  761:         if (m.startTime !== undefined) thinkingStartTime.value = m.startTime;
  762:         break;
  763:       case "updateThinkingDuration":
  764:         thinkingDuration.value = m.duration;
  765:         break;
  766:       case "addToolCall":
  767:         activeToolCalls.value.push(m.toolCall);
  768:         break;
  769:       case "updateToolCall": {
  770:         const tc = activeToolCalls.value.find((t) => t.id === m.id);
  771:         if (tc) Object.assign(tc, m.updates);
  772:         break;
  773:       }
  774:       case "addNestedToolCall": {
  775:         const parent = activeToolCalls.value.find((t) => t.id === m.parentId);
  776:         if (parent) {
  777:           if (!parent.nestedToolCalls) parent.nestedToolCalls = [];
  778:           parent.nestedToolCalls.push(m.toolCall);
  779:         }
  780:         break;
  781:       }
  782:       case "updateNestedToolCall": {
  783:         const parentTc = activeToolCalls.value.find((t) => t.id === m.parentId);
  784:         const nested = parentTc?.nestedToolCalls?.find((t) => t.id === m.childId);
  785:         if (nested) Object.assign(nested, m.updates);
  786:         break;
  787:       }
  788:       case "appendToolDelta": {
  789:         const tcDelta = activeToolCalls.value.find((t) => t.id === m.id);
  790:         if (tcDelta) tcDelta.output = (tcDelta.output || "") + m.delta;
  791:         break;
  792:       }
  793:       case "updateToolProgress": {
  794:         const tcProgress = activeToolCalls.value.find((t) => t.id === m.id);
  795:         if (tcProgress) tcProgress.progress = m.progress;
  796:         break;
  797:       }
  798:       case "pushMessage":
  799:         messages.value = replaceMessageById(messages.value, m.message);
  800:         if (m.message.role === "assistant") {
  801:           logToolCollapseTrace("chat-store", "pushMessage", {
  802:             messageId: m.message.id,
  803:             contentLen: m.message.content.length,
  804:             toolCallCount: m.message.toolCalls?.length ?? 0,
  805:             thinkingLen: m.message.thinkingContent?.length ?? 0,
  806:           });
  807:         }
  808:         break;
  809:       case "upsertMessage":
  810:         messages.value = replaceMessageById(messages.value, m.message);
  811:         break;
  812:       case "upsertUserMessage":
  813:         messages.value = mergeUserMessage(messages.value, m.message);
  814:         break;
  815:       case "replaceMessages":
  816:         messages.value = hydrateMessages(m.messages);
  817:         break;
  818:       case "pushToolResults":
  819:         logToolCollapseTrace("chat-store", "pushToolResults", {
  820:           toolCallCount: activeToolCalls.value.length,
  821:           toolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
  822:           targetToolCallIds: m.toolCallIds ?? null,
  823:         });
  824:         {
  825:           const targetIds = m.toolCallIds ? new Set(m.toolCallIds) : null;
  826:           const sourceToolCalls = targetIds
  827:             ? activeToolCalls.value.filter((toolCall) => targetIds.has(toolCall.id))
  828:             : activeToolCalls.value;
  829:           for (const message of buildToolResultMessages(sourceToolCalls)) {
  830:             messages.value = replaceMessageById(messages.value, message);
  831:           }
  832:         }
  833:         break;
  834:       case "resetRound":
  835:         logToolCollapseTrace("chat-store", "resetRound", {
  836:           rawStreamLen: rawStreamText.value.length,
  837:           streamingLen: streamingText.value.length,
  838:           thinkingLen: streamingThinking.value.length,
  839:           activeToolCallCount: activeToolCalls.value.length,
  840:           activeToolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
  841:         });
  842:         resetStreamAnim();
  843:         streamingThinking.value = "";
  844:         streamingTextOrder.value = 0;
  845:         thinkingOrder.value = 0;
  846:         liveRenderParts.value = [];
  847:         thinkingStartTime.value = 0;
  848:         thinkingDuration.value = 0;
  849:         isThinking.value = false;
  850:         activeToolCalls.value = [];
  851:         break;
  852:       case "resetRoundKeepToolCalls":
  853:         logToolCollapseTrace("chat-store", "resetRoundKeepToolCalls", {
  854:           rawStreamLen: rawStreamText.value.length,
  855:           streamingLen: streamingText.value.length,
  856:           thinkingLen: streamingThinking.value.length,
  857:           activeToolCallCount: activeToolCalls.value.length,
  858:           activeToolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
  859:         });
  860:         resetStreamAnim();
  861:         streamingThinking.value = "";
  862:         streamingTextOrder.value = 0;
  863:         thinkingOrder.value = 0;
  864:         liveRenderParts.value = [];
  865:         thinkingStartTime.value = 0;
  866:         thinkingDuration.value = 0;
  867:         isThinking.value = false;
  868:         break;
  869:       case "clearPendingInputs":
  870:         pendingQuestion.value = null;
  871:         pendingToolConfirms.value = [];
  872:         break;
  873:       case "clearPendingInput":
  874:         if (pendingQuestion.value?.questionId === m.questionId) {
  875:           pendingQuestion.value = null;
  876:         }
  877:         pendingToolConfirms.value = pendingToolConfirms.value.filter(
  878:           (item) => item.questionId !== m.questionId,
  879:         );
  880:         break;
  881:       case "updateUsage":
  882:         tokenUsage.value = m.usage;
  883:         break;
  884:       case "setQuestion":
  885:         pendingQuestion.value = m.question;
  886:         break;
  887:       case "enqueueToolConfirm": {
  888:         const next = pendingToolConfirms.value.filter((item) => item.questionId !== m.confirm.questionId);
  889:         next.push(m.confirm);
  890:         pendingToolConfirms.value = next;
  891:         break;
  892:       }
  893:       case "addUndoable":
  894:         undoableMessageIds.value.add(m.messageId);
  895:         break;
  896:       case "setTodos":
  897:         todos.value = m.todos;
  898:         if (activeSessionId.value) {
  899:           sessionLatestTodoRunIds.value.set(activeSessionId.value, m.runId);
  900:         }
  901:         todoWriteVersion.value += 1;
  902:         if (m.todos.length > 0 && useDisplaySettings().state.todoAutoOpen) {
  903:           setTodoPanelVisible(true);
  904:         } else {
  905:           persistTodoPanelState();
  906:         }
  907:         break;
  908:       case "setStreaming":
  909:         if (isStreaming.value !== m.value) {
  910:           logToolCollapseTrace("chat-store", "setStreaming", {
  911:             previous: isStreaming.value,
  912:             next: m.value,
  913:           });
  914:         }
  915:         isStreaming.value = m.value;
  916:         break;
  917:       case "setCompacting":
  918:         isCompacting.value = m.value;
  919:         break;
  920:       case "canvasAutoOpen":
  921:         if (streamReplayDepth === 0) {
  922:           canvasAutoOpenCallback?.(m.toolCallId, m.spec);
  923:         }
  924:         break;
  925:     }
  926:   }
  927: 
  928:   // -- Stream event handler --
  929:   function handleStreamEvent(event: StreamEvent): boolean {
  930:     if (event.type === "runStart") {
  931:       const closedRunId = closedRunIds.get(event.sessionId);
  932:       if (closedRunId && closedRunId === event.runId) {
  933:         logChatStreamDebug("ignoring runStart for already-closed run", {
  934:           sessionId: event.sessionId,
  935:           runId: event.runId,
  936:           closedRunId,
  937:         });
  938:         return false;
  939:       }
  940: 
  941:       const expectedRunId = sessionRunIds.value.get(event.sessionId);
  942:       if (expectedRunId && expectedRunId !== event.runId) {
  943:         logChatStreamDebug("ignoring runStart with unexpected run id", {
  944:           sessionId: event.sessionId,
  945:           runId: event.runId,
  946:           expectedRunId,
  947:         });
  948:         return false;
  949:       }
  950: 
````

### src/stores/chat.ts:1120-1210

````ts
 1120:       return true;
 1121:     }
 1122: 
 1123:     // Build current state snapshot for reducer
 1124:     const state = {
 1125:       messages: messages.value,
 1126:       streamingText: streamingText.value,
 1127:       rawStreamText: rawStreamText.value,
 1128:       streamingThinking: streamingThinking.value,
 1129:       streamSequence: streamSequence.value,
 1130:       streamingTextOrder: streamingTextOrder.value,
 1131:       thinkingOrder: thinkingOrder.value,
 1132:       liveRenderParts: liveRenderParts.value,
 1133:       isStreaming: isStreaming.value,
 1134:       isCompacting: isCompacting.value,
 1135:       isThinking: isThinking.value,
 1136:       thinkingStartTime: thinkingStartTime.value,
 1137:       thinkingDuration: thinkingDuration.value,
 1138:       activeToolCalls: activeToolCalls.value,
 1139:       tokenUsage: tokenUsage.value,
 1140:       todos: todos.value,
 1141:       showTodoPanel: showTodoPanel.value,
 1142:       pendingQuestion: pendingQuestion.value,
 1143:       pendingToolConfirms: pendingToolConfirms.value,
 1144:       undoableMessageIds: undoableMessageIds.value,
 1145:     };
 1146: 
 1147:     switch (event.type) {
 1148:       case "textDelta":
 1149:         logToolCollapseTrace("chat-store", "handleStreamEvent:textDelta", {
 1150:           sessionId: event.sessionId,
 1151:           runId: event.runId,
 1152:           textLen: event.text.length,
 1153:           textPreview: previewTraceText(event.text, 48),
 1154:           activeToolCallCount: activeToolCalls.value.length,
 1155:           isStreaming: isStreaming.value,
 1156:         });
 1157:         break;
 1158:       case "toolCallRoundDone":
 1159:         logToolCollapseTrace("chat-store", "handleStreamEvent:toolCallRoundDone", {
 1160:           sessionId: event.sessionId,
 1161:           runId: event.runId,
 1162:           messageId: event.messageId,
 1163:           fullTextLen: event.fullText.length,
 1164:           toolCallCount: event.toolCalls.length,
 1165:           activeToolCallCount: activeToolCalls.value.length,
 1166:         });
 1167:         break;
 1168:       case "done":
 1169:         logToolCollapseTrace("chat-store", "handleStreamEvent:done", {
 1170:           sessionId: event.sessionId,
 1171:           runId: event.runId,
 1172:           messageId: event.messageId,
 1173:           fullTextLen: event.fullText.length,
 1174:           rawStreamLen: rawStreamText.value.length,
 1175:           streamingLen: streamingText.value.length,
 1176:           activeToolCallCount: activeToolCalls.value.length,
 1177:         });
 1178:         break;
 1179:     }
 1180: 
 1181:     const mutations = reduceStreamEvent(state, event);
 1182:     for (const m of mutations) {
 1183:       applyMutation(m);
 1184:     }
 1185: 
 1186:     // Push stream errors to global notification
 1187:     if (event.type === "error") {
 1188:       useNotificationStore().addNotice("error", event.error.message, {
 1189:         code: event.error.code,
 1190:         operation: "chat",
 1191:       });
 1192:       void loadSessionState(event.sessionId);
 1193:     }
 1194: 
 1195:     if (event.type === "done") {
 1196:       // Save plan artifact on successful plan completion
 1197:       if (
 1198:         pendingPlanRun.value &&
 1199:         pendingPlanRun.value.runId === event.runId &&
 1200:         pendingPlanRun.value.sessionId === event.sessionId
 1201:       ) {
 1202:         sessionService.savePlanArtifact(
 1203:           pendingPlanRun.value.sessionId,
 1204:           pendingPlanRun.value.agentId,
 1205:           pendingPlanRun.value.requestText,
 1206:           event.fullText,
 1207:         ).catch((e) => console.warn("[plan] save artifact failed:", e));
 1208:       }
 1209:     }
 1210: 
````

### src/components/ChatView.vue:140-180

````vue
  140: 
  141: async function onChatDiffLfsPulled() {
  142:   const payload = chatChangesStore.inlineDiffPayload;
  143:   if (!payload) return;
  144:   try {
  145:     const updated = await refetchDiffByKey(payload.key);
  146:     if (updated) chatChangesStore.inlineDiffPayload = updated;
  147:   } catch (e) {
  148:     console.error("[ChatView] refetch after LFS pull failed:", e);
  149:   }
  150: }
  151: 
  152: const props = defineProps<{
  153:   messages: ChatMessage[];
  154:   streamingText: string;
  155:   streamingTextOrder?: number;
  156:   isStreaming: boolean;
  157:   isCompacting: boolean;
  158:   isThinking: boolean;
  159:   hasThinking: boolean;
  160:   thinkingText: string;
  161:   thinkingOrder?: number;
  162:   thinkingDuration: number;
  163:   liveRenderParts?: AssistantRenderPart[];
  164:   activeToolCalls: ToolCallDisplay[];
  165:   agents: AgentInfo[];
  166:   selectedAgentId: string;
  167:   agentLocked: boolean;
  168:   models: ModelOption[];
  169:   selectedModelId: string;
  170:   codexTransport?: CodexTransportMode;
  171:   effort: EffortLevel;
  172:   effortSupported: boolean;
  173:   effortLevels: EffortLevel[];
  174:   tokenUsage: TokenUsage;
  175:   pendingQuestion: PendingQuestion | null;
  176:   pendingToolConfirms: PendingToolConfirm[];
  177:   sessions: SessionSummary[];
  178:   activeSessionId: string | null;
  179:   unityConnected?: boolean;
  180:   unityPluginStatus?: "missing" | "outdated" | null;
````

### src/components/ChatView.vue:820-910

````vue
  820:   );
  821: }
  822: 
  823: const showWelcomeState = computed(
  824:   () =>
  825:     !props.messages.some((message) => hasRenderableTranscriptMessage(message))
  826:     && !hasStreamingContent.value
  827:     && !props.isThinking
  828:     && !props.hasThinking
  829:     && !isWaitingForResponse.value,
  830: );
  831: 
  832: const pendingRestoreSessionId = ref<string | null>(null);
  833: const pendingRestoreMessagesRef = ref<ChatMessage[] | null>(null);
  834: const toolHandoffViewportQuiet = ref(false);
  835: let suppressScrollCapture = false;
  836: let activeToolViewportAnchor: LiveScrollAnchorSnapshot | null = null;
  837: let toolViewportAnchorFrame = 0;
  838: const displayedStreamingText = ref("");
  839: let pendingStreamingText = "";
  840: let streamingTextFlushTimer: ReturnType<typeof setTimeout> | null = null;
  841: const STREAMING_TEXT_RENDER_DELAY_MS = 80;
  842: const STREAM_END_SCROLL_SETTLE_MS = 320;
  843: 
  844: function clearStreamingTextFlushTimer() {
  845:   if (!streamingTextFlushTimer) return;
  846:   clearTimeout(streamingTextFlushTimer);
  847:   streamingTextFlushTimer = null;
  848: }
  849: 
  850: function flushDisplayedStreamingText() {
  851:   logToolCollapseTrace("chat-view", "flushDisplayedStreamingText", {
  852:     pendingLen: pendingStreamingText.length,
  853:     pendingPreview: previewTraceText(pendingStreamingText, 64),
  854:     previousDisplayedLen: displayedStreamingText.value.length,
  855:   });
  856:   displayedStreamingText.value = pendingStreamingText;
  857:   streamingTextFlushTimer = null;
  858: }
  859: 
  860: watch(
  861:   () => props.streamingText,
  862:   (nextText, previousText = "") => {
  863:     pendingStreamingText = nextText;
  864:     logToolCollapseTrace("chat-view", "sourceStreamingTextChanged", {
  865:       previousLen: previousText.length,
  866:       nextLen: nextText.length,
  867:       displayedLen: displayedStreamingText.value.length,
  868:       hasFlushTimer: !!streamingTextFlushTimer,
  869:       nextPreview: nextText ? previewTraceText(nextText, 64) : "",
  870:     });
  871:     if (!nextText || nextText.length < displayedStreamingText.value.length) {
  872:       clearStreamingTextFlushTimer();
  873:       logToolCollapseTrace("chat-view", "syncDisplayedStreamingTextImmediately", {
  874:         reason: !nextText ? "empty" : "shrinking",
  875:         nextLen: nextText.length,
  876:         previousDisplayedLen: displayedStreamingText.value.length,
  877:       });
  878:       displayedStreamingText.value = nextText;
  879:       return;
  880:     }
  881:     if (streamingTextFlushTimer) {
  882:       logToolCollapseTrace("chat-view", "skipStreamingTextReschedule", {
  883:         pendingLen: pendingStreamingText.length,
  884:         displayedLen: displayedStreamingText.value.length,
  885:       });
  886:       return;
  887:     }
  888:     logToolCollapseTrace("chat-view", "scheduleDisplayedStreamingTextFlush", {
  889:       delayMs: STREAMING_TEXT_RENDER_DELAY_MS,
  890:       nextLen: nextText.length,
  891:       displayedLen: displayedStreamingText.value.length,
  892:     });
  893:     streamingTextFlushTimer = setTimeout(flushDisplayedStreamingText, STREAMING_TEXT_RENDER_DELAY_MS);
  894:   },
  895:   { immediate: true },
  896: );
  897: 
  898: function readMessageMetrics(el: HTMLElement) {
  899:   return {
  900:     scrollTop: el.scrollTop,
  901:     clientHeight: el.clientHeight,
  902:     scrollHeight: el.scrollHeight,
  903:   };
  904: }
  905: 
  906: function getMessagesElement() {
  907:   return transcriptRef.value?.getScrollElement() ?? null;
  908: }
  909: 
  910: function getMessagesContentElement() {
````

### src/components/ChatView.vue:990-1145

````vue
  990:   });
  991: 
  992:   if (!restored) {
  993:     clearToolViewportAnchor();
  994:   }
  995:   return restored;
  996: }
  997: 
  998: function handleToolViewportAnchorStart(anchor: HTMLElement) {
  999:   const el = getMessagesElement();
 1000:   if (!el || !el.contains(anchor)) return;
 1001: 
 1002:   scrollToBottomScheduler.cancel();
 1003:   preserveScrollAnchorScheduler.cancel();
 1004:   streamEndScrollScheduler.cancel();
 1005:   clearToolViewportAnchorFrame();
 1006:   activeToolViewportAnchor = captureLiveScrollAnchor(el, anchor);
 1007:   restoreToolViewportAnchor();
 1008: }
 1009: 
 1010: function handleToolViewportAnchorEnd(anchor: HTMLElement) {
 1011:   if (!activeToolViewportAnchor || activeToolViewportAnchor.anchor !== anchor) return;
 1012: 
 1013:   restoreToolViewportAnchor();
 1014:   clearToolViewportAnchorFrame();
 1015:   toolViewportAnchorFrame = requestViewportFrame(() => {
 1016:     toolViewportAnchorFrame = 0;
 1017:     restoreToolViewportAnchor();
 1018:     activeToolViewportAnchor = null;
 1019:   });
 1020: }
 1021: 
 1022: function setMessagesScrollTop(scrollTop: number, sessionId: string | null = props.activeSessionId) {
 1023:   runProgrammaticScrollUpdate((el) => {
 1024:     el.scrollTop = scrollTop;
 1025:   }, sessionId);
 1026: }
 1027: 
 1028: function restoreMessagesScrollState(
 1029:   state: ReturnType<typeof chatStore.getSessionScrollState>,
 1030:   sessionId: string | null = props.activeSessionId,
 1031: ) {
 1032:   const el = getMessagesElement();
 1033:   if (!el) return;
 1034: 
 1035:   const nextScrollTop = resolveSessionScrollTop(readMessageMetrics(el), state);
 1036:   runProgrammaticScrollUpdate((element) => {
 1037:     if (!restoreScrollAnchor(element, state)) {
 1038:       element.scrollTop = nextScrollTop;
 1039:     }
 1040:   }, sessionId);
 1041: }
 1042: 
 1043: function isPendingSessionRestoreAwaitingMessages() {
 1044:   const targetSessionId = pendingRestoreSessionId.value;
 1045:   return !!targetSessionId
 1046:     && targetSessionId === props.activeSessionId
 1047:     && pendingRestoreMessagesRef.value === props.messages;
 1048: }
 1049: 
 1050: function scrollToBottomNow(force = false) {
 1051:   if (isPendingSessionRestoreAwaitingMessages()) return;
 1052: 
 1053:   const el = getMessagesElement();
 1054:   if (!el) return;
 1055: 
 1056:   const metrics = readMessageMetrics(el);
 1057:   const remembered = props.activeSessionId ? chatStore.getSessionScrollState(props.activeSessionId) : null;
 1058:   if (!shouldAutoScrollToBottom({ force, metrics, remembered })) {
 1059:     return;
 1060:   }
 1061: 
 1062:   setMessagesScrollTop(resolveSessionScrollTop(metrics, { mode: "bottom" }));
 1063: }
 1064: 
 1065: const scrollToBottomScheduler = createCoalescedScrollScheduler((force) => {
 1066:   nextTick(() => {
 1067:     scrollToBottomNow(force);
 1068:   });
 1069: });
 1070: 
 1071: const preserveScrollAnchorScheduler = createCoalescedScrollScheduler(() => {
 1072:   nextTick(() => {
 1073:     if (isPendingSessionRestoreAwaitingMessages()) return;
 1074: 
 1075:     const sessionId = props.activeSessionId;
 1076:     const remembered = sessionId ? chatStore.getSessionScrollState(sessionId) : null;
 1077:     if (!remembered || remembered.mode === "bottom") return;
 1078:     restoreMessagesScrollState(remembered, sessionId);
 1079:   });
 1080: });
 1081: 
 1082: function scrollToBottom(force = false) {
 1083:   scrollToBottomScheduler.schedule(force);
 1084: }
 1085: 
 1086: function preserveScrollAnchor() {
 1087:   preserveScrollAnchorScheduler.schedule();
 1088: }
 1089: 
 1090: const streamEndScrollScheduler = createSettledScrollScheduler(
 1091:   () => scrollToBottom(true),
 1092:   STREAM_END_SCROLL_SETTLE_MS,
 1093: );
 1094: 
 1095: function handleToolHandoffQuietChange(quiet: boolean) {
 1096:   logToolCollapseTrace("chat-view", "toolHandoffQuietChange", {
 1097:     quiet,
 1098:     displayedStreamingLen: displayedStreamingText.value.length,
 1099:     isStreaming: props.isStreaming,
 1100:   });
 1101:   toolHandoffViewportQuiet.value = quiet;
 1102: }
 1103: 
 1104: watch(toolHandoffViewportQuiet, (quiet, previousQuiet) => {
 1105:   if (quiet) {
 1106:     scrollToBottomScheduler.cancel();
 1107:     preserveScrollAnchorScheduler.cancel();
 1108:     streamEndScrollScheduler.cancel();
 1109:     return;
 1110:   }
 1111:   if (previousQuiet) {
 1112:     reconcileViewport();
 1113:   }
 1114: });
 1115: 
 1116: function reconcileViewport(forceBottom = false) {
 1117:   if (toolHandoffViewportQuiet.value) return;
 1118:   if (restoreToolViewportAnchor()) return;
 1119:   if (pendingRestoreSessionId.value && pendingRestoreSessionId.value === props.activeSessionId) {
 1120:     restorePendingSessionScroll();
 1121:     return;
 1122:   }
 1123: 
 1124:   const el = getMessagesElement();
 1125:   if (!el) return;
 1126: 
 1127:   const remembered = props.activeSessionId ? chatStore.getSessionScrollState(props.activeSessionId) : null;
 1128:   if (shouldAutoScrollToBottom({ force: forceBottom, metrics: readMessageMetrics(el), remembered })) {
 1129:     scrollToBottom(forceBottom);
 1130:     return;
 1131:   }
 1132: 
 1133:   preserveScrollAnchor();
 1134: }
 1135: 
 1136: function settleStreamEndScroll() {
 1137:   if (toolHandoffViewportQuiet.value) return;
 1138:   const el = getMessagesElement();
 1139:   if (!el) return;
 1140: 
 1141:   const metrics = readMessageMetrics(el);
 1142:   const remembered = props.activeSessionId ? chatStore.getSessionScrollState(props.activeSessionId) : null;
 1143:   if (!shouldAutoScrollToBottom({ metrics, remembered })) {
 1144:     preserveScrollAnchor();
 1145:     return;
````

### src/components/ChatView.vue:1260-1385

````vue
 1260:     return;
 1261:   }
 1262: 
 1263:   scheduleTranscriptResizeReconcile("observer");
 1264: }
 1265: 
 1266: function flushPendingTranscriptResizeReconcile(reason: string) {
 1267:   if (!transcriptResizeReconcilePending) return;
 1268:   scheduleTranscriptResizeReconcile(reason);
 1269: }
 1270: 
 1271: function disconnectTranscriptResizeObserver() {
 1272:   cancelTranscriptResizeReconcileFrame();
 1273:   transcriptResizeReconcilePending = false;
 1274:   transcriptResizeObserver?.disconnect();
 1275:   transcriptResizeObserver = null;
 1276: }
 1277: 
 1278: function connectTranscriptResizeObserver() {
 1279:   disconnectTranscriptResizeObserver();
 1280:   if (typeof ResizeObserver === "undefined") return;
 1281: 
 1282:   const scrollEl = getMessagesElement();
 1283:   const contentEl = getMessagesContentElement();
 1284:   if (!scrollEl && !contentEl) return;
 1285:   transcriptObservedViewportWidth = readTranscriptViewportWidth();
 1286: 
 1287:   transcriptResizeObserver = createAnimationFrameResizeObserver(handleTranscriptResize);
 1288:   if (!transcriptResizeObserver) return;
 1289: 
 1290:   if (scrollEl) {
 1291:     transcriptResizeObserver.observe(scrollEl);
 1292:   }
 1293:   if (contentEl && contentEl !== scrollEl) {
 1294:     transcriptResizeObserver.observe(contentEl);
 1295:   }
 1296: }
 1297: 
 1298: watch(
 1299:   () => props.activeSessionId,
 1300:   (nextSessionId, previousSessionId) => {
 1301:     clearToolViewportAnchor();
 1302:     scrollToBottomScheduler.cancel();
 1303:     streamEndScrollScheduler.cancel();
 1304:     preserveScrollAnchorScheduler.cancel();
 1305:     toolHandoffViewportQuiet.value = false;
 1306:     if (previousSessionId) {
 1307:       rememberScrollForSession(previousSessionId);
 1308:     }
 1309: 
 1310:     const shouldRestoreImmediately = !!nextSessionId && previousSessionId === null && !showWelcomeState.value;
 1311:     pendingRestoreSessionId.value = nextSessionId;
 1312:     pendingRestoreMessagesRef.value = nextSessionId && !shouldRestoreImmediately ? props.messages : null;
 1313:     void restoreComposerDraft(nextSessionId ?? null);
 1314:     if (shouldRestoreImmediately) {
 1315:       restorePendingSessionScroll({ defer: true });
 1316:     }
 1317:   },
 1318:   { flush: "sync" },
 1319: );
 1320: 
 1321: watch(
 1322:   () => props.messages,
 1323:   (messages) => {
 1324:     if (!pendingRestoreSessionId.value || pendingRestoreSessionId.value !== props.activeSessionId) return;
 1325:     if (messages === pendingRestoreMessagesRef.value) return;
 1326:     restorePendingSessionScroll();
 1327:   },
 1328:   { flush: "post" },
 1329: );
 1330: 
 1331: watch(
 1332:   () => props.messages,
 1333:   (messages, previous) => {
 1334:     if (messages === previous || pendingRestoreSessionId.value) return;
 1335:     reconcileViewport();
 1336:   },
 1337:   { flush: "post" },
 1338: );
 1339: watch(() => props.messages.length, () => reconcileViewport());
 1340: watch(() => displayedStreamingText.value, () => reconcileViewport());
 1341: watch(() => props.activeToolCalls, () => reconcileViewport(), { deep: true });
 1342: watch(
 1343:   () => props.isStreaming,
 1344:   (nextStreaming, previousStreaming) => {
 1345:     logToolCollapseTrace("chat-view", "isStreamingChanged", {
 1346:       previous: previousStreaming,
 1347:       next: nextStreaming,
 1348:       displayedStreamingLen: displayedStreamingText.value.length,
 1349:       sourceStreamingLen: props.streamingText.length,
 1350:       activeToolCallCount: props.activeToolCalls.length,
 1351:       quiet: toolHandoffViewportQuiet.value,
 1352:     });
 1353:     if (nextStreaming) {
 1354:       streamEndScrollScheduler.cancel();
 1355:       return;
 1356:     }
 1357:     if (previousStreaming) {
 1358:       settleStreamEndScroll();
 1359:     }
 1360:   },
 1361: );
 1362: watch(isWaitingForResponse, (v) => { if (v) reconcileViewport(); });
 1363: watch(() => props.pendingQuestion?.questionId ?? null, (q) => {
 1364:   if (q) reconcileViewport();
 1365: });
 1366: watch(() => props.pendingToolConfirms.map((item) => item.questionId).join(":"), (value) => {
 1367:   if (value) reconcileViewport();
 1368: });
 1369: 
 1370: const keepBatchToolConfirmLayout = ref(false);
 1371: 
 1372: watch(
 1373:   () => [props.activeSessionId, props.pendingToolConfirms.map((item) => item.questionId).join(":")],
 1374:   ([sessionId], previous = []) => {
 1375:     const [prevSessionId] = previous;
 1376:     const count = props.pendingToolConfirms.length;
 1377:     if (sessionId !== prevSessionId) {
 1378:       keepBatchToolConfirmLayout.value = count > 1;
 1379:       return;
 1380:     }
 1381:     if (count === 0) {
 1382:       keepBatchToolConfirmLayout.value = false;
 1383:       return;
 1384:     }
 1385:     if (count > 1) {
````

### src/components/ChatView.vue:1680-1745

````vue
 1680:       class="chat-view"
 1681:       :class="{ 'is-vertical-layout': isVerticalLayout }"
 1682:     >
 1683:       <SessionCompactPicker
 1684:         v-if="showSessionCompactPicker"
 1685:         :sessions="sessions"
 1686:         :active-session-id="activeSessionId"
 1687:         :streaming-session-ids="streamingSessionIds"
 1688:         :show-expand-panel-button="sessionPanelCollapsed && !isVerticalLayout"
 1689:         @select-session="emit('selectSession', $event)"
 1690:         @new-chat="handleNewChatRequest"
 1691:         @expand-panel="setSessionPanelCollapsed(false)"
 1692:       />
 1693:       <div class="chat-main">
 1694:         <ChatTranscript
 1695:           ref="transcriptRef"
 1696:           variant="session"
 1697:           :session-key="activeSessionId || NEW_CHAT_DRAFT_KEY"
 1698:           :messages="messages"
 1699:           :streaming-text="displayedStreamingText"
 1700:           :streaming-text-order="streamingTextOrder"
 1701:           :is-streaming="isStreaming"
 1702:           :is-compacting="isCompacting"
 1703:           :is-thinking="isThinking"
 1704:           :has-thinking="hasThinking"
 1705:           :thinking-order="thinkingOrder"
 1706:           :thinking-duration="thinkingDuration"
 1707:           :live-render-parts="liveRenderParts"
 1708:           :active-tool-calls="activeToolCalls"
 1709:           user-label="You"
 1710:           assistant-label="Locus"
 1711:           :handoff-label="t('chat.transcript.handoff')"
 1712:           :waiting-label="t('chat.transcript.waiting')"
 1713:           :compacting-label="t('chat.transcript.compacting')"
 1714:           :compacted-label="t('chat.transcript.compacted')"
 1715:           :thinking-active-label="t('chat.transcript.thinking')"
 1716:           :thought-duration-label="t('chat.transcript.thoughtDuration', '{0}')"
 1717:           :thought-moment-label="t('chat.transcript.thoughtMoment')"
 1718:           enable-intent-badges
 1719:           show-user-images
 1720:           user-content-mode="asset"
 1721:           @scroll="onMessagesScroll"
 1722:           @content-click="handleContentClick"
 1723:           @content-contextmenu="handleContentContextMenu"
 1724:           @open-thinking="emit('openThinking', $event)"
 1725:           @open-image="openLightbox"
 1726:           @apply-knowledge-proposal="chatStore.applyKnowledgeProposal"
 1727:           @ignore-knowledge-proposal="chatStore.ignoreKnowledgeProposal"
 1728:           @tool-handoff-quiet-change="handleToolHandoffQuietChange"
 1729:           @tool-viewport-anchor-start="handleToolViewportAnchorStart"
 1730:           @tool-viewport-anchor-end="handleToolViewportAnchorEnd"
 1731:         >
 1732:         </ChatTranscript>
 1733:         <div v-if="showWelcomeState" class="chat-empty-overlay">
 1734:           <div class="empty-state">
 1735:             <div class="empty-icon">L</div>
 1736:             <div class="empty-title">Locus</div>
 1737:             <div class="empty-subtitle">{{ t("onboarding.welcome.subtitle") }}</div>
 1738:           </div>
 1739:         </div>
 1740:       </div>
 1741: 
 1742:     <div
 1743:       v-if="(pendingQuestion && !isViewingSubagent) || showBatchToolConfirmCard || showSingleToolConfirmCard || (isPlanDone && !isViewingSubagent) || (isPlanStreaming && !isViewingSubagent)"
 1744:       class="chat-pending-stack"
 1745:       @wheel="handleBottomPanelWheel"
````

### src/components/chat/EmbeddedChatPane.vue:1-220

````vue
    1: <script setup lang="ts">
    2: import { computed, nextTick, onMounted, onUnmounted, ref, useSlots, watch } from "vue";
    3: import type {
    4:   ChatComposerSendPayload,
    5:   ChatMessage,
    6:   PendingQuestion,
    7:   PendingToolConfirm,
    8:   SkillManifest,
    9:   ToolCallDisplay,
   10:   AssistantRenderPart,
   11: } from "../../types";
   12: import AskUserCard from "./AskUserCard.vue";
   13: import ToolConfirmCard from "./ToolConfirmCard.vue";
   14: import ToolConfirmBatchCard from "./ToolConfirmBatchCard.vue";
   15: import ChatTranscript from "./ChatTranscript.vue";
   16: import RichChatInput from "./RichChatInput.vue";
   17: import { forwardWheelToElement } from "../../composables/chatWheelPassthrough";
   18: import {
   19:   captureScrollAnchor,
   20:   captureLiveScrollAnchor,
   21:   captureSessionScrollState,
   22:   resolveSessionScrollTop,
   23:   restoreLiveScrollAnchor,
   24:   restoreScrollAnchor,
   25:   type LiveScrollAnchorSnapshot,
   26:   type SessionScrollState,
   27: } from "../../composables/chatScrollState";
   28: import {
   29:   createCoalescedScrollScheduler,
   30:   createSettledScrollScheduler,
   31:   shouldAutoScrollToBottom,
   32: } from "../../composables/chatViewStability";
   33: import {
   34:   createAnimationFrameResizeObserver,
   35:   type ResizeObserverHandle,
   36: } from "../../composables/resizeObserver";
   37: import { t } from "../../i18n";
   38: 
   39: interface MetaRow {
   40:   label: string;
   41:   value: string;
   42: }
   43: 
   44: const props = withDefaults(defineProps<{
   45:   title?: string;
   46:   subtitle?: string;
   47:   metaRows?: MetaRow[];
   48:   messages: ChatMessage[];
   49:   streamingText: string;
   50:   streamingTextOrder?: number;
   51:   thinkingText?: string;
   52:   thinkingOrder?: number;
   53:   isStreaming: boolean;
   54:   isCompacting?: boolean;
   55:   isThinking: boolean;
   56:   thinkingDuration?: number;
   57:   liveRenderParts?: AssistantRenderPart[];
   58:   activeToolCalls: ToolCallDisplay[];
   59:   pendingQuestion?: PendingQuestion | null;
   60:   pendingToolConfirms?: PendingToolConfirm[];
   61:   toolConfirmLayoutKey?: string | null;
   62:   inputValue: string;
   63:   placeholder?: string;
   64:   emptyTitle?: string;
   65:   emptyHint?: string;
   66:   errorMessage?: string | null;
   67:   disabled?: boolean;
   68:   sendLabel?: string;
   69:   cancelLabel?: string;
   70:   userLabel?: string;
   71:   assistantLabel?: string;
   72:   thinkingLabel?: string;
   73:   waitingLabel?: string;
   74:   compactingLabel?: string;
   75:   compactedLabel?: string;
   76:   thoughtDurationLabel?: string;
   77:   thoughtMomentLabel?: string;
   78:   runningLabel?: string;
   79:   selectedAgentId?: string;
   80:   skills?: SkillManifest[];
   81:   enableIntentBadges?: boolean;
   82:   showUserImages?: boolean;
   83:   userContentMode?: "plain" | "asset";
   84: }>(), {
   85:   subtitle: "",
   86:   metaRows: () => [],
   87:   thinkingText: "",
   88:   thinkingDuration: 0,
   89:   isCompacting: false,
   90:   pendingQuestion: null,
   91:   pendingToolConfirms: () => [],
   92:   toolConfirmLayoutKey: null,
   93:   placeholder: "",
   94:   emptyTitle: "",
   95:   emptyHint: "",
   96:   errorMessage: null,
   97:   disabled: false,
   98:   sendLabel: "",
   99:   cancelLabel: "",
  100:   userLabel: "",
  101:   assistantLabel: "Locus",
  102:   thinkingLabel: "",
  103:   waitingLabel: "",
  104:   compactingLabel: "",
  105:   compactedLabel: "",
  106:   thoughtDurationLabel: "",
  107:   thoughtMomentLabel: "",
  108:   runningLabel: "",
  109:   selectedAgentId: "",
  110:   skills: () => [],
  111:   enableIntentBadges: false,
  112:   showUserImages: false,
  113:   userContentMode: "plain",
  114: });
  115: 
  116: const emit = defineEmits<{
  117:   (e: "update:inputValue", value: string): void;
  118:   (e: "send", payload: ChatComposerSendPayload): void;
  119:   (e: "cancel"): void;
  120:   (e: "clear"): void;
  121:   (e: "answerQuestion", value: string): void;
  122:   (e: "answerToolConfirm", questionId: string, answer: string): void;
  123:   (e: "answerAllToolConfirms", questionIds: string[], answer: string): void;
  124:   (e: "applyKnowledgeProposal", proposalId: string): void;
  125:   (e: "ignoreKnowledgeProposal", proposalId: string): void;
  126: }>();
  127: 
  128: const slots = useSlots();
  129: const transcriptRef = ref<InstanceType<typeof ChatTranscript> | null>(null);
  130: const hasHeader = computed(() => !!props.title || !!props.subtitle || !!slots["header-actions"]);
  131: const hasComposerStart = computed(() => !!slots["composer-start"]);
  132: const hasComposerActions = computed(() => !!slots["composer-actions"]);
  133: const effectiveSendLabel = computed(() => props.sendLabel || t("common.send"));
  134: const effectiveCancelLabel = computed(() => props.cancelLabel || t("common.cancel"));
  135: const effectiveUserLabel = computed(() => props.userLabel || t("chat.embedded.user"));
  136: const effectiveThinkingLabel = computed(() => props.thinkingLabel || t("chat.embedded.thinking"));
  137: const effectiveWaitingLabel = computed(() => props.waitingLabel || props.runningLabel || t("chat.embedded.running"));
  138: const effectiveCompactingLabel = computed(() => props.compactingLabel || t("chat.transcript.compacting"));
  139: const effectiveCompactedLabel = computed(() => props.compactedLabel || t("chat.transcript.compacted"));
  140: const effectiveThoughtDurationLabel = computed(() =>
  141:   props.thoughtDurationLabel || t("chat.transcript.thoughtDuration", "{0}"),
  142: );
  143: const effectiveThoughtMomentLabel = computed(() =>
  144:   props.thoughtMomentLabel || t("chat.transcript.thoughtMoment"),
  145: );
  146: const viewportStates = new Map<string, SessionScrollState>();
  147: let suppressScrollCapture = false;
  148: let transcriptResizeObserver: ResizeObserverHandle | null = null;
  149: const toolHandoffViewportQuiet = ref(false);
  150: let activeToolViewportAnchor: LiveScrollAnchorSnapshot | null = null;
  151: let toolViewportAnchorFrame = 0;
  152: const STREAM_END_SCROLL_SETTLE_MS = 320;
  153: 
  154: function updateInput(value: string) {
  155:   emit("update:inputValue", value);
  156: }
  157: 
  158: function getViewportStateKey(key = props.toolConfirmLayoutKey) {
  159:   return key?.trim() || "__embedded__";
  160: }
  161: 
  162: function getTranscriptElement() {
  163:   return transcriptRef.value?.getScrollElement() ?? null;
  164: }
  165: 
  166: function getTranscriptContentElement() {
  167:   return transcriptRef.value?.getContentElement?.() ?? null;
  168: }
  169: 
  170: function readTranscriptMetrics(el: HTMLElement) {
  171:   return {
  172:     scrollTop: el.scrollTop,
  173:     clientHeight: el.clientHeight,
  174:     scrollHeight: el.scrollHeight,
  175:   };
  176: }
  177: 
  178: function captureViewportState(el: HTMLElement): SessionScrollState {
  179:   return captureSessionScrollState(readTranscriptMetrics(el), captureScrollAnchor(el));
  180: }
  181: 
  182: function rememberViewportState(key = getViewportStateKey()) {
  183:   const el = getTranscriptElement();
  184:   if (!el) return;
  185:   viewportStates.set(key, captureViewportState(el));
  186: }
  187: 
  188: function runProgrammaticScrollUpdate(update: (el: HTMLElement) => void, key = getViewportStateKey()) {
  189:   const el = getTranscriptElement();
  190:   if (!el) return;
  191: 
  192:   suppressScrollCapture = true;
  193:   update(el);
  194:   viewportStates.set(key, captureViewportState(el));
  195: 
  196:   requestAnimationFrame(() => {
  197:     suppressScrollCapture = false;
  198:   });
  199: }
  200: 
  201: function requestViewportFrame(callback: () => void): number {
  202:   if (typeof requestAnimationFrame === "function") {
  203:     return requestAnimationFrame(() => callback());
  204:   }
  205:   return window.setTimeout(callback, 16);
  206: }
  207: 
  208: function cancelViewportFrame(handle: number) {
  209:   if (typeof cancelAnimationFrame === "function") {
  210:     cancelAnimationFrame(handle);
  211:     return;
  212:   }
  213:   window.clearTimeout(handle);
  214: }
  215: 
  216: function clearToolViewportAnchorFrame() {
  217:   if (!toolViewportAnchorFrame) return;
  218:   cancelViewportFrame(toolViewportAnchorFrame);
  219:   toolViewportAnchorFrame = 0;
  220: }
````

### src/components/chat/EmbeddedChatPane.vue:220-460

````vue
  220: }
  221: 
  222: function clearToolViewportAnchor() {
  223:   clearToolViewportAnchorFrame();
  224:   activeToolViewportAnchor = null;
  225: }
  226: 
  227: function restoreToolViewportAnchor() {
  228:   const anchorState = activeToolViewportAnchor;
  229:   const el = getTranscriptElement();
  230:   if (!anchorState || !el) return false;
  231:   if (!el.contains(anchorState.anchor)) {
  232:     clearToolViewportAnchor();
  233:     return false;
  234:   }
  235: 
  236:   suppressScrollCapture = true;
  237:   const restored = restoreLiveScrollAnchor(el, anchorState);
  238:   if (restored) {
  239:     viewportStates.set(getViewportStateKey(), captureViewportState(el));
  240:   }
  241: 
  242:   requestViewportFrame(() => {
  243:     suppressScrollCapture = false;
  244:   });
  245: 
  246:   if (!restored) {
  247:     clearToolViewportAnchor();
  248:   }
  249:   return restored;
  250: }
  251: 
  252: function handleToolViewportAnchorStart(anchor: HTMLElement) {
  253:   const el = getTranscriptElement();
  254:   if (!el || !el.contains(anchor)) return;
  255: 
  256:   scrollToBottomScheduler.cancel();
  257:   preserveScrollAnchorScheduler.cancel();
  258:   streamEndScrollScheduler.cancel();
  259:   clearToolViewportAnchorFrame();
  260:   activeToolViewportAnchor = captureLiveScrollAnchor(el, anchor);
  261:   restoreToolViewportAnchor();
  262: }
  263: 
  264: function handleToolViewportAnchorEnd(anchor: HTMLElement) {
  265:   if (!activeToolViewportAnchor || activeToolViewportAnchor.anchor !== anchor) return;
  266: 
  267:   restoreToolViewportAnchor();
  268:   clearToolViewportAnchorFrame();
  269:   toolViewportAnchorFrame = requestViewportFrame(() => {
  270:     toolViewportAnchorFrame = 0;
  271:     restoreToolViewportAnchor();
  272:     activeToolViewportAnchor = null;
  273:   });
  274: }
  275: 
  276: function scrollToBottomNow(force = false) {
  277:   const el = getTranscriptElement();
  278:   if (!el) return;
  279: 
  280:   const remembered = viewportStates.get(getViewportStateKey()) ?? null;
  281:   if (!shouldAutoScrollToBottom({ force, metrics: readTranscriptMetrics(el), remembered })) {
  282:     return;
  283:   }
  284: 
  285:   runProgrammaticScrollUpdate((element) => {
  286:     element.scrollTop = resolveSessionScrollTop(readTranscriptMetrics(element), { mode: "bottom" });
  287:   });
  288: }
  289: 
  290: const scrollToBottomScheduler = createCoalescedScrollScheduler((force) => {
  291:   nextTick(() => {
  292:     scrollToBottomNow(force);
  293:   });
  294: });
  295: 
  296: const preserveScrollAnchorScheduler = createCoalescedScrollScheduler(() => {
  297:   nextTick(() => {
  298:     const remembered = viewportStates.get(getViewportStateKey()) ?? null;
  299:     if (!remembered || remembered.mode === "bottom") return;
  300: 
  301:     const el = getTranscriptElement();
  302:     if (!el) return;
  303: 
  304:     const nextScrollTop = resolveSessionScrollTop(readTranscriptMetrics(el), remembered);
  305:     runProgrammaticScrollUpdate((element) => {
  306:       if (!restoreScrollAnchor(element, remembered)) {
  307:         element.scrollTop = nextScrollTop;
  308:       }
  309:     });
  310:   });
  311: });
  312: 
  313: const streamEndScrollScheduler = createSettledScrollScheduler(
  314:   () => scrollToBottom(true),
  315:   STREAM_END_SCROLL_SETTLE_MS,
  316: );
  317: 
  318: function handleToolHandoffQuietChange(quiet: boolean) {
  319:   toolHandoffViewportQuiet.value = quiet;
  320: }
  321: 
  322: watch(toolHandoffViewportQuiet, (quiet, previousQuiet) => {
  323:   if (quiet) {
  324:     scrollToBottomScheduler.cancel();
  325:     preserveScrollAnchorScheduler.cancel();
  326:     streamEndScrollScheduler.cancel();
  327:     return;
  328:   }
  329:   if (previousQuiet) {
  330:     reconcileViewport();
  331:   }
  332: });
  333: 
  334: function scrollToBottom(force = false) {
  335:   scrollToBottomScheduler.schedule(force);
  336: }
  337: 
  338: function preserveScrollAnchor() {
  339:   preserveScrollAnchorScheduler.schedule();
  340: }
  341: 
  342: function reconcileViewport(forceBottom = false) {
  343:   if (toolHandoffViewportQuiet.value) return;
  344:   if (restoreToolViewportAnchor()) return;
  345:   const el = getTranscriptElement();
  346:   if (!el) return;
  347: 
  348:   const remembered = viewportStates.get(getViewportStateKey()) ?? null;
  349:   if (shouldAutoScrollToBottom({ force: forceBottom, metrics: readTranscriptMetrics(el), remembered })) {
  350:     scrollToBottom(forceBottom);
  351:     return;
  352:   }
  353: 
  354:   preserveScrollAnchor();
  355: }
  356: 
  357: function restoreViewportStateForKey(key = getViewportStateKey()) {
  358:   const remembered = viewportStates.get(key) ?? null;
  359:   if (!remembered) {
  360:     scrollToBottom(true);
  361:     return;
  362:   }
  363: 
  364:   const el = getTranscriptElement();
  365:   if (!el) return;
  366: 
  367:   const nextScrollTop = resolveSessionScrollTop(readTranscriptMetrics(el), remembered);
  368:   runProgrammaticScrollUpdate((element) => {
  369:     if (!restoreScrollAnchor(element, remembered)) {
  370:       element.scrollTop = nextScrollTop;
  371:     }
  372:   }, key);
  373: }
  374: 
  375: function handleTranscriptScroll() {
  376:   if (suppressScrollCapture) return;
  377:   scrollToBottomScheduler.cancel();
  378:   preserveScrollAnchorScheduler.cancel();
  379:   streamEndScrollScheduler.cancel();
  380:   rememberViewportState();
  381: }
  382: 
  383: function disconnectTranscriptResizeObserver() {
  384:   transcriptResizeObserver?.disconnect();
  385:   transcriptResizeObserver = null;
  386: }
  387: 
  388: function connectTranscriptResizeObserver() {
  389:   disconnectTranscriptResizeObserver();
  390:   if (typeof ResizeObserver === "undefined") return;
  391: 
  392:   const scrollEl = getTranscriptElement();
  393:   const contentEl = getTranscriptContentElement();
  394:   if (!scrollEl && !contentEl) return;
  395: 
  396:   transcriptResizeObserver = createAnimationFrameResizeObserver(() => {
  397:     if (suppressScrollCapture || toolHandoffViewportQuiet.value) return;
  398:     if (restoreToolViewportAnchor()) return;
  399:     reconcileViewport();
  400:   });
  401:   if (!transcriptResizeObserver) return;
  402: 
  403:   if (scrollEl) {
  404:     transcriptResizeObserver.observe(scrollEl);
  405:   }
  406:   if (contentEl && contentEl !== scrollEl) {
  407:     transcriptResizeObserver.observe(contentEl);
  408:   }
  409: }
  410: 
  411: function handleBottomPanelWheel(event: WheelEvent) {
  412:   forwardWheelToElement(event, getTranscriptElement());
  413: }
  414: 
  415: const keepBatchToolConfirmLayout = ref(false);
  416: 
  417: watch(
  418:   () => props.toolConfirmLayoutKey ?? "",
  419:   (nextKey, previousKey) => {
  420:     clearToolViewportAnchor();
  421:     toolHandoffViewportQuiet.value = false;
  422:     rememberViewportState(getViewportStateKey(previousKey));
  423:     preserveScrollAnchorScheduler.cancel();
  424:     streamEndScrollScheduler.cancel();
  425:     nextTick(() => {
  426:       restoreViewportStateForKey(getViewportStateKey(nextKey));
  427:       connectTranscriptResizeObserver();
  428:     });
  429:   },
  430:   { flush: "pre" },
  431: );
  432: 
  433: watch(
  434:   () => [props.toolConfirmLayoutKey ?? "", props.pendingToolConfirms.map((item) => item.questionId).join(":")],
  435:   ([layoutKey], previous = []) => {
  436:     const [prevLayoutKey] = previous;
  437:     const count = props.pendingToolConfirms.length;
  438:     if (layoutKey !== prevLayoutKey) {
  439:       keepBatchToolConfirmLayout.value = count > 1;
  440:       return;
  441:     }
  442:     if (count === 0) {
  443:       keepBatchToolConfirmLayout.value = false;
  444:       return;
  445:     }
  446:     if (count > 1) {
  447:       keepBatchToolConfirmLayout.value = true;
  448:     }
  449:   },
  450:   { immediate: true },
  451: );
  452: 
  453: const showBatchToolConfirmCard = computed(() =>
  454:   props.pendingToolConfirms.length > 0
  455:   && (keepBatchToolConfirmLayout.value || props.pendingToolConfirms.length > 1),
  456: );
  457: 
  458: const showSingleToolConfirmCard = computed(() =>
  459:   props.pendingToolConfirms.length === 1
  460:   && !showBatchToolConfirmCard.value,
````

### src/components/chat/EmbeddedChatPane.vue:520-650

````vue
  520:   scrollToBottomScheduler.cancel();
  521:   preserveScrollAnchorScheduler.cancel();
  522:   streamEndScrollScheduler.cancel();
  523:   clearToolViewportAnchor();
  524:   disconnectTranscriptResizeObserver();
  525: });
  526: </script>
  527: 
  528: <template>
  529:   <div class="embedded-chat-pane">
  530:     <div v-if="hasHeader" class="embedded-chat-header">
  531:       <div class="embedded-chat-heading">
  532:         <div class="embedded-chat-title">{{ title }}</div>
  533:         <div v-if="subtitle" class="embedded-chat-subtitle">{{ subtitle }}</div>
  534:       </div>
  535:       <div class="embedded-chat-header-actions">
  536:         <slot name="header-actions" />
  537:       </div>
  538:     </div>
  539: 
  540:     <div v-if="metaRows.length > 0" class="embedded-chat-context">
  541:       <div v-for="row in metaRows" :key="row.label" class="embedded-chat-context-row">
  542:         <span class="embedded-chat-context-label">{{ row.label }}</span>
  543:         <span class="embedded-chat-context-value" :title="row.value">{{ row.value }}</span>
  544:       </div>
  545:     </div>
  546: 
  547:     <div v-if="errorMessage" class="embedded-chat-error">{{ errorMessage }}</div>
  548: 
  549:     <ChatTranscript
  550:       ref="transcriptRef"
  551:       variant="embedded"
  552:       :session-key="getViewportStateKey()"
  553:       :messages="messages"
  554:       :streaming-text="streamingText"
  555:       :streaming-text-order="streamingTextOrder"
  556:       :is-streaming="isStreaming"
  557:       :is-compacting="isCompacting"
  558:       :is-thinking="isThinking"
  559:       :thinking-text="thinkingText"
  560:       :thinking-order="thinkingOrder"
  561:       :thinking-duration="thinkingDuration"
  562:       :live-render-parts="liveRenderParts"
  563:       :active-tool-calls="activeToolCalls"
  564:       :empty-title="emptyTitle"
  565:       :empty-hint="emptyHint"
  566:       :user-label="effectiveUserLabel"
  567:       :assistant-label="assistantLabel"
  568:       :waiting-label="effectiveWaitingLabel"
  569:       :compacting-label="effectiveCompactingLabel"
  570:       :compacted-label="effectiveCompactedLabel"
  571:       :thinking-active-label="effectiveThinkingLabel"
  572:       :thought-duration-label="effectiveThoughtDurationLabel"
  573:       :thought-moment-label="effectiveThoughtMomentLabel"
  574:       :enable-intent-badges="enableIntentBadges"
  575:       :show-user-images="showUserImages"
  576:       :user-content-mode="userContentMode"
  577:       @scroll="handleTranscriptScroll"
  578:       @apply-knowledge-proposal="emit('applyKnowledgeProposal', $event)"
  579:       @ignore-knowledge-proposal="emit('ignoreKnowledgeProposal', $event)"
  580:       @tool-handoff-quiet-change="handleToolHandoffQuietChange"
  581:       @tool-viewport-anchor-start="handleToolViewportAnchorStart"
  582:       @tool-viewport-anchor-end="handleToolViewportAnchorEnd"
  583:     />
  584: 
  585:     <div class="embedded-chat-bottom">
  586:       <div
  587:         v-if="pendingQuestion || showBatchToolConfirmCard || showSingleToolConfirmCard"
  588:         class="embedded-chat-panels"
  589:         @wheel="handleBottomPanelWheel"
  590:       >
  591:         <AskUserCard
  592:           v-if="pendingQuestion"
  593:           :question="pendingQuestion"
  594:           @answer="emit('answerQuestion', $event)"
  595:         />
  596:         <ToolConfirmBatchCard
  597:           v-if="showBatchToolConfirmCard"
  598:           :tool-confirms="pendingToolConfirms"
  599:           @answer="emit('answerToolConfirm', $event.questionId, $event.answer)"
  600:           @answer-many="emit('answerAllToolConfirms', $event.questionIds, $event.answer)"
  601:         />
  602:         <ToolConfirmCard
  603:           v-else-if="showSingleToolConfirmCard"
  604:           :tool-confirm="pendingToolConfirms[0]!"
  605:           @answer="emit('answerToolConfirm', pendingToolConfirms[0]!.questionId, $event)"
  606:         />
  607:       </div>
  608: 
  609:       <RichChatInput
  610:         :model-value="inputValue"
  611:         :selected-agent-id="selectedAgentId"
  612:         :skills="skills"
  613:         :placeholder="placeholder"
  614:         :disabled="disabled"
  615:         :is-streaming="isStreaming"
  616:         :send-label="effectiveSendLabel"
  617:         :cancel-label="effectiveCancelLabel"
  618:         @update:model-value="updateInput"
  619:         @send="emit('send', $event)"
  620:         @clear="emit('clear')"
  621:         @cancel="emit('cancel')"
  622:       >
  623:         <template v-if="hasComposerStart" #top-start>
  624:           <slot name="composer-start" />
  625:         </template>
  626:         <template v-if="hasComposerActions" #top-end>
  627:           <slot name="composer-actions" />
  628:         </template>
  629:       </RichChatInput>
  630:     </div>
  631:   </div>
  632: </template>
  633: 
  634: <style scoped>
  635: .embedded-chat-pane {
  636:   height: 100%;
  637:   display: flex;
  638:   flex-direction: column;
  639:   min-height: 0;
  640:   background: color-mix(in srgb, var(--sidebar-bg) 76%, var(--panel-bg));
  641: }
  642: 
  643: .embedded-chat-header {
  644:   display: flex;
  645:   align-items: flex-start;
  646:   justify-content: space-between;
  647:   gap: 10px;
  648:   padding: 10px 12px;
  649:   border-bottom: 1px solid var(--border-color);
  650:   background: color-mix(in srgb, var(--sidebar-bg) 90%, var(--panel-bg));
````

### src-tauri/src/commands/mod.rs:1-170

````rust
    1: pub mod asset;
    2: mod auth;
    3: mod canvas;
    4: mod diff;
    5: mod fonts;
    6: mod git;
    7: mod knowledge;
    8: mod log;
    9: mod plan;
   10: mod ref_graph;
   11: mod session;
   12: mod skill;
   13: mod storage;
   14: mod system;
   15: mod undo;
   16: mod unity_embed;
   17: mod update;
   18: mod workspace;
   19: 
   20: use serde::{Deserialize, Serialize};
   21: 
   22: use crate::error::AppError;
   23: 
   24: #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
   25: #[serde(rename_all = "lowercase")]
   26: pub enum ToolCallOutcome {
   27:     Done,
   28:     Error,
   29:     Interrupted,
   30: }
   31: 
   32: #[derive(Debug, Clone, Serialize, Deserialize)]
   33: #[serde(tag = "type", rename_all = "camelCase")]
   34: pub enum StreamEvent {
   35:     #[serde(rename_all = "camelCase")]
   36:     RunStart { session_id: String },
   37:     #[serde(rename_all = "camelCase")]
   38:     UserMessage {
   39:         session_id: String,
   40:         message: crate::session::models::ChatMessage,
   41:     },
   42:     #[serde(rename_all = "camelCase")]
   43:     TextDelta {
   44:         session_id: String,
   45:         text: String,
   46:         #[serde(default, skip_serializing_if = "Option::is_none")]
   47:         order: Option<u32>,
   48:         #[serde(default, skip_serializing_if = "Option::is_none")]
   49:         part_id: Option<String>,
   50:         #[serde(default, skip_serializing_if = "Option::is_none")]
   51:         render_seq: Option<u32>,
   52:     },
   53:     #[serde(rename_all = "camelCase")]
   54:     ThinkingDelta {
   55:         session_id: String,
   56:         text: String,
   57:         #[serde(default, skip_serializing_if = "Option::is_none")]
   58:         order: Option<u32>,
   59:         #[serde(default, skip_serializing_if = "Option::is_none")]
   60:         part_id: Option<String>,
   61:         #[serde(default, skip_serializing_if = "Option::is_none")]
   62:         render_seq: Option<u32>,
   63:     },
   64:     #[serde(rename_all = "camelCase")]
   65:     ToolCallStart {
   66:         session_id: String,
   67:         tool_call_id: String,
   68:         tool_name: String,
   69:         arguments: String,
   70:         #[serde(default, skip_serializing_if = "Option::is_none")]
   71:         order: Option<u32>,
   72:         #[serde(default, skip_serializing_if = "Option::is_none")]
   73:         part_id: Option<String>,
   74:         #[serde(default, skip_serializing_if = "Option::is_none")]
   75:         render_seq: Option<u32>,
   76:     },
   77:     #[serde(rename_all = "camelCase")]
   78:     ToolCallDone {
   79:         session_id: String,
   80:         tool_call_id: String,
   81:         tool_name: String,
   82:         output: String,
   83:         outcome: ToolCallOutcome,
   84:     },
   85:     #[serde(rename_all = "camelCase")]
   86:     ToolCallDelta {
   87:         session_id: String,
   88:         tool_call_id: String,
   89:         delta: String,
   90:     },
   91:     #[serde(rename_all = "camelCase")]
   92:     ToolCallProgress {
   93:         session_id: String,
   94:         tool_call_id: String,
   95:         title: String,
   96:         info: String,
   97:         progress: Option<f32>,
   98:         state: String,
   99:     },
  100:     #[serde(rename_all = "camelCase")]
  101:     SubagentToolCallStart {
  102:         session_id: String,
  103:         parent_tool_call_id: String,
  104:         tool_call_id: String,
  105:         tool_name: String,
  106:         arguments: String,
  107:         #[serde(default, skip_serializing_if = "Option::is_none")]
  108:         order: Option<u32>,
  109:         #[serde(default, skip_serializing_if = "Option::is_none")]
  110:         part_id: Option<String>,
  111:         #[serde(default, skip_serializing_if = "Option::is_none")]
  112:         render_seq: Option<u32>,
  113:     },
  114:     #[serde(rename_all = "camelCase")]
  115:     SubagentToolCallDone {
  116:         session_id: String,
  117:         parent_tool_call_id: String,
  118:         tool_call_id: String,
  119:         tool_name: String,
  120:         output: String,
  121:         outcome: ToolCallOutcome,
  122:     },
  123:     #[serde(rename_all = "camelCase")]
  124:     ToolCallRoundDone {
  125:         session_id: String,
  126:         message_id: String,
  127:         full_text: String,
  128:         tool_calls: Vec<crate::session::models::ToolCallInfo>,
  129:         #[serde(default, skip_serializing_if = "Option::is_none")]
  130:         content_order: Option<u32>,
  131:         #[serde(default, skip_serializing_if = "Option::is_none")]
  132:         thinking_order: Option<u32>,
  133:         #[serde(default, skip_serializing_if = "Option::is_none")]
  134:         render_parts: Option<Vec<crate::session::models::AssistantRenderPart>>,
  135:     },
  136:     #[serde(rename_all = "camelCase")]
  137:     Done {
  138:         session_id: String,
  139:         message_id: String,
  140:         full_text: String,
  141:         #[serde(default, skip_serializing_if = "Option::is_none")]
  142:         content_order: Option<u32>,
  143:         #[serde(default, skip_serializing_if = "Option::is_none")]
  144:         thinking_order: Option<u32>,
  145:         #[serde(default, skip_serializing_if = "Option::is_none")]
  146:         render_parts: Option<Vec<crate::session::models::AssistantRenderPart>>,
  147:     },
  148:     #[serde(rename_all = "camelCase")]
  149:     KnowledgeProposal {
  150:         session_id: String,
  151:         message: crate::session::models::ChatMessage,
  152:     },
  153:     #[serde(rename_all = "camelCase")]
  154:     UsageUpdate {
  155:         session_id: String,
  156:         input_tokens: u32,
  157:         output_tokens: u32,
  158:         cache_read_tokens: u32,
  159:         cache_write_tokens: u32,
  160:         total_input_tokens: u64,
  161:         total_output_tokens: u64,
  162:         total_cache_read_tokens: u64,
  163:         total_cache_write_tokens: u64,
  164:         total_cost_usd: f64,
  165:         priced_rounds: u64,
  166:         context_tokens: u32,
  167:         context_limit: u32,
  168:     },
  169:     #[serde(rename_all = "camelCase")]
  170:     AskUser {
````

### src-tauri/src/session/models.rs:1-220

````rust
    1: use serde::{Deserialize, Serialize};
    2: 
    3: #[derive(Debug, Clone, Serialize, Deserialize)]
    4: #[serde(rename_all = "camelCase")]
    5: pub struct SessionSummary {
    6:     pub id: String,
    7:     pub title: String,
    8:     pub agent_id: Option<String>,
    9:     pub session_type: String,
   10:     pub parent_session_id: Option<String>,
   11:     pub updated_at: i64,
   12:     #[serde(skip_serializing_if = "Option::is_none")]
   13:     pub runtime_status: Option<SessionRuntimeStatus>,
   14: }
   15: 
   16: #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
   17: #[serde(rename_all = "snake_case")]
   18: pub enum SessionRuntimeStatus {
   19:     Running,
   20:     Queued,
   21:     Starting,
   22:     WaitingInput,
   23:     Cancelling,
   24:     Error,
   25: }
   26: 
   27: #[derive(Debug, Clone, Serialize, Deserialize)]
   28: #[serde(rename_all = "camelCase")]
   29: pub struct SessionDetail {
   30:     pub id: String,
   31:     pub title: String,
   32:     pub agent_id: Option<String>,
   33:     pub session_type: String,
   34:     pub parent_session_id: Option<String>,
   35:     #[serde(skip_serializing_if = "Option::is_none")]
   36:     pub latest_completed_run_id: Option<String>,
   37:     pub created_at: i64,
   38:     pub updated_at: i64,
   39:     pub messages: Vec<ChatMessage>,
   40: }
   41: 
   42: #[derive(Debug, Clone, Serialize, Deserialize)]
   43: #[serde(rename_all = "camelCase")]
   44: pub struct SessionRunSummary {
   45:     pub run_id: String,
   46:     pub session_id: String,
   47:     pub status: String,
   48:     pub started_at: i64,
   49:     pub updated_at: i64,
   50:     #[serde(skip_serializing_if = "Option::is_none")]
   51:     pub finished_at: Option<i64>,
   52:     #[serde(skip_serializing_if = "Option::is_none")]
   53:     pub error_message: Option<String>,
   54: }
   55: 
   56: #[derive(Debug, Clone, Serialize, Deserialize)]
   57: #[serde(rename_all = "camelCase")]
   58: pub struct SessionEventRecord {
   59:     pub session_id: String,
   60:     pub run_id: String,
   61:     pub seq: i64,
   62:     pub event_type: String,
   63:     pub payload: serde_json::Value,
   64:     pub created_at: i64,
   65: }
   66: 
   67: #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
   68: #[serde(rename_all = "lowercase")]
   69: pub enum MessageRole {
   70:     User,
   71:     Assistant,
   72:     Tool,
   73: }
   74: 
   75: impl MessageRole {
   76:     pub fn as_str(&self) -> &str {
   77:         match self {
   78:             MessageRole::User => "user",
   79:             MessageRole::Assistant => "assistant",
   80:             MessageRole::Tool => "tool",
   81:         }
   82:     }
   83: 
   84:     pub fn from_str(s: &str) -> Result<Self, String> {
   85:         match s {
   86:             "user" => Ok(MessageRole::User),
   87:             "assistant" => Ok(MessageRole::Assistant),
   88:             "tool" => Ok(MessageRole::Tool),
   89:             _ => Err(format!("Unknown role: {}", s)),
   90:         }
   91:     }
   92: }
   93: 
   94: #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
   95: #[serde(rename_all = "snake_case")]
   96: pub enum ServerToolKind {
   97:     WebSearch,
   98: }
   99: 
  100: #[derive(Debug, Clone, Serialize, Deserialize)]
  101: #[serde(rename_all = "camelCase")]
  102: pub struct ToolCallInfo {
  103:     pub id: String,
  104:     pub name: String,
  105:     pub arguments: String,
  106:     #[serde(default, skip_serializing_if = "Option::is_none")]
  107:     pub order: Option<u32>,
  108:     #[serde(default, skip_serializing_if = "Option::is_none")]
  109:     pub server_tool: Option<ServerToolKind>,
  110:     /// Pre-computed output for server tools (e.g. web_search) that don't need local execution.
  111:     #[serde(skip_serializing_if = "Option::is_none")]
  112:     pub server_tool_output: Option<String>,
  113:     #[serde(default, skip_serializing_if = "Option::is_none")]
  114:     pub outcome: Option<crate::commands::ToolCallOutcome>,
  115:     #[serde(default, skip_serializing_if = "Option::is_none")]
  116:     pub recorded_output: Option<String>,
  117:     #[serde(default, skip_serializing_if = "Option::is_none")]
  118:     pub nested_tool_calls: Option<Vec<ToolCallInfo>>,
  119: }
  120: 
  121: impl ToolCallInfo {
  122:     pub fn is_server_tool(&self) -> bool {
  123:         self.server_tool.is_some() || self.server_tool_output.is_some()
  124:     }
  125: }
  126: 
  127: #[derive(Debug, Clone, Serialize, Deserialize)]
  128: #[serde(rename_all = "camelCase")]
  129: pub struct RenderOrderKey {
  130:     pub run_id: String,
  131:     pub seq: u32,
  132: }
  133: 
  134: #[derive(Debug, Clone, Serialize, Deserialize)]
  135: #[serde(tag = "kind", rename_all = "camelCase")]
  136: pub enum AssistantRenderPart {
  137:     #[serde(rename_all = "camelCase")]
  138:     Thinking {
  139:         id: String,
  140:         order: RenderOrderKey,
  141:         content: String,
  142:         #[serde(default, skip_serializing_if = "Option::is_none")]
  143:         active: Option<bool>,
  144:         #[serde(default, skip_serializing_if = "Option::is_none")]
  145:         duration: Option<u32>,
  146:         #[serde(default, skip_serializing_if = "Option::is_none")]
  147:         signature: Option<String>,
  148:     },
  149:     #[serde(rename_all = "camelCase")]
  150:     Text {
  151:         id: String,
  152:         order: RenderOrderKey,
  153:         content: String,
  154:     },
  155:     #[serde(rename_all = "camelCase")]
  156:     ToolCall {
  157:         id: String,
  158:         order: RenderOrderKey,
  159:         tool_call: ToolCallInfo,
  160:     },
  161:     #[serde(rename_all = "camelCase")]
  162:     KnowledgeProposal {
  163:         id: String,
  164:         order: RenderOrderKey,
  165:         message: Box<ChatMessage>,
  166:     },
  167: }
  168: 
  169: #[derive(Debug, Clone, Serialize, Deserialize)]
  170: #[serde(rename_all = "camelCase")]
  171: pub struct ImageData {
  172:     pub data: String,
  173:     pub mime_type: String,
  174: }
  175: 
  176: #[derive(Debug, Clone, Serialize, Deserialize)]
  177: #[serde(rename_all = "camelCase")]
  178: pub struct AssetRefData {
  179:     pub path: String,
  180:     pub kind: String,
  181:     #[serde(default, skip_serializing_if = "Option::is_none")]
  182:     pub name: Option<String>,
  183:     #[serde(default, skip_serializing_if = "Option::is_none")]
  184:     pub type_label: Option<String>,
  185:     #[serde(default, skip_serializing_if = "Option::is_none")]
  186:     pub source: Option<String>,
  187: }
  188: 
  189: #[derive(Debug, Clone, Serialize, Deserialize)]
  190: #[serde(rename_all = "camelCase")]
  191: pub struct UserIntentSkill {
  192:     pub dir_name: String,
  193:     pub source: String,
  194:     pub name: String,
  195: }
  196: 
  197: #[derive(Debug, Clone, Serialize, Deserialize)]
  198: #[serde(rename_all = "camelCase")]
  199: pub struct UserIntentPayload {
  200:     pub kind: String,
  201:     pub mode: String,
  202:     #[serde(default)]
  203:     pub skills: Vec<UserIntentSkill>,
  204:     #[serde(default, skip_serializing_if = "Option::is_none")]
  205:     pub client_message_id: Option<String>,
  206: }
  207: 
  208: #[derive(Debug, Clone, Serialize, Deserialize)]
  209: pub struct TodoItem {
  210:     pub content: String,
  211:     pub status: String,
  212:     pub priority: String,
  213: }
  214: 
  215: #[derive(Debug, Clone, Serialize, Deserialize)]
  216: #[serde(rename_all = "camelCase")]
  217: pub struct TodoSnapshot {
  218:     pub items: Vec<TodoItem>,
  219:     pub latest_run_id: Option<String>,
  220: }
````

### src-tauri/src/session/models.rs:220-430

````rust
  220: }
  221: 
  222: #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  223: #[serde(rename_all = "snake_case")]
  224: pub enum KnowledgeProposalVerify {
  225:     None,
  226:     Required,
  227: }
  228: 
  229: #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  230: #[serde(rename_all = "snake_case")]
  231: pub enum KnowledgeProposalStatus {
  232:     Pending,
  233:     Applying,
  234:     Applied,
  235:     Invalidated,
  236:     Stale,
  237: }
  238: 
  239: #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  240: #[serde(rename_all = "snake_case")]
  241: pub enum KnowledgeProposalItemKind {
  242:     Memory,
  243:     #[serde(alias = "wiki")]
  244:     Knowledge,
  245: }
  246: 
  247: #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  248: #[serde(rename_all = "snake_case")]
  249: pub enum KnowledgeProposalItemMode {
  250:     Replace,
  251:     CreateSource,
  252:     UpdateSource,
  253: }
  254: 
  255: #[derive(Debug, Clone, Serialize, Deserialize)]
  256: #[serde(rename_all = "camelCase")]
  257: pub struct KnowledgeProposalItem {
  258:     pub kind: KnowledgeProposalItemKind,
  259:     pub mode: KnowledgeProposalItemMode,
  260:     pub target: String,
  261:     pub draft: String,
  262: }
  263: 
  264: #[derive(Debug, Clone, Serialize, Deserialize)]
  265: #[serde(rename_all = "camelCase")]
  266: pub struct KnowledgeProposal {
  267:     pub proposal_id: String,
  268:     pub status: KnowledgeProposalStatus,
  269:     pub confidence: f32,
  270:     pub verify: KnowledgeProposalVerify,
  271:     pub est_tokens: u32,
  272:     #[serde(default)]
  273:     pub items: Vec<KnowledgeProposalItem>,
  274:     pub created_at: i64,
  275:     pub updated_at: i64,
  276: }
  277: 
  278: #[derive(Debug, Clone, Serialize, Deserialize)]
  279: #[serde(rename_all = "camelCase")]
  280: pub struct ChatMessage {
  281:     pub id: String,
  282:     pub role: MessageRole,
  283:     pub content: String,
  284:     pub created_at: i64,
  285:     #[serde(skip_serializing_if = "Option::is_none")]
  286:     pub prompt_prefix: Option<String>,
  287:     #[serde(skip_serializing_if = "Option::is_none")]
  288:     pub prompt_suffix: Option<String>,
  289:     #[serde(skip_serializing_if = "Option::is_none")]
  290:     pub response_id: Option<String>,
  291:     #[serde(skip_serializing_if = "Option::is_none")]
  292:     pub content_order: Option<u32>,
  293:     #[serde(skip_serializing_if = "Option::is_none")]
  294:     pub thinking_order: Option<u32>,
  295:     #[serde(skip_serializing_if = "Option::is_none")]
  296:     pub tool_calls: Option<Vec<ToolCallInfo>>,
  297:     #[serde(skip_serializing_if = "Option::is_none")]
  298:     pub tool_call_id: Option<String>,
  299:     #[serde(skip_serializing_if = "Option::is_none")]
  300:     pub images: Option<Vec<ImageData>>,
  301:     #[serde(skip_serializing_if = "Option::is_none")]
  302:     pub asset_refs: Option<Vec<AssetRefData>>,
  303:     #[serde(skip_serializing_if = "Option::is_none")]
  304:     pub thinking_content: Option<String>,
  305:     #[serde(skip_serializing_if = "Option::is_none")]
  306:     pub thinking_duration: Option<u32>,
  307:     #[serde(skip_serializing_if = "Option::is_none")]
  308:     pub thinking_signature: Option<String>,
  309:     #[serde(skip_serializing_if = "Option::is_none")]
  310:     pub knowledge_proposal: Option<KnowledgeProposal>,
  311:     #[serde(skip_serializing_if = "Option::is_none")]
  312:     pub render_parts: Option<Vec<AssistantRenderPart>>,
  313: }
  314: 
````

### src-tauri/src/session/store.rs:260-320

````rust
  260: struct MessageMetadata {
  261:     #[serde(skip_serializing_if = "Option::is_none")]
  262:     knowledge_proposal: Option<KnowledgeProposal>,
  263:     #[serde(skip_serializing_if = "Option::is_none")]
  264:     response_id: Option<String>,
  265:     #[serde(skip_serializing_if = "Option::is_none")]
  266:     response_request: Option<serde_json::Value>,
  267:     #[serde(skip_serializing_if = "Option::is_none")]
  268:     content_order: Option<u32>,
  269:     #[serde(skip_serializing_if = "Option::is_none")]
  270:     thinking_order: Option<u32>,
  271:     #[serde(skip_serializing_if = "Option::is_none")]
  272:     render_parts: Option<Vec<AssistantRenderPart>>,
  273: }
  274: 
  275: fn message_metadata_json(
  276:     knowledge_proposal: Option<&KnowledgeProposal>,
  277:     response_id: Option<&str>,
  278:     response_request: Option<&serde_json::Value>,
  279:     content_order: Option<u32>,
  280:     thinking_order: Option<u32>,
  281:     render_parts: Option<&[AssistantRenderPart]>,
  282: ) -> Result<Option<String>, String> {
  283:     let metadata = MessageMetadata {
  284:         knowledge_proposal: knowledge_proposal.cloned(),
  285:         response_id: response_id.map(|value| value.to_string()),
  286:         response_request: response_request.cloned(),
  287:         content_order,
  288:         thinking_order,
  289:         render_parts: render_parts.map(|value| value.to_vec()),
  290:     };
  291:     if metadata.knowledge_proposal.is_none()
  292:         && metadata.response_id.is_none()
  293:         && metadata.response_request.is_none()
  294:         && metadata.content_order.is_none()
  295:         && metadata.thinking_order.is_none()
  296:         && metadata.render_parts.is_none()
  297:     {
  298:         return Ok(None);
  299:     }
  300:     serde_json::to_string(&metadata)
  301:         .map(Some)
  302:         .map_err(|e| format!("Failed to serialize message metadata: {}", e))
  303: }
  304: 
  305: fn merge_prompt_prefixes(carried: &str, existing: Option<&str>) -> String {
  306:     let carried_trimmed = carried.trim();
  307:     if carried_trimmed.is_empty() {
  308:         return existing.unwrap_or_default().to_string();
  309:     }
  310: 
  311:     let existing_value = existing.unwrap_or_default();
  312:     let existing_trimmed = existing_value.trim();
  313:     if existing_trimmed.is_empty() {
  314:         return carried_trimmed.to_string();
  315:     }
  316:     if existing_trimmed == carried_trimmed || existing_trimmed.starts_with(carried_trimmed) {
  317:         return existing_value.to_string();
  318:     }
  319: 
  320:     format!("{}\n\n{}", carried_trimmed, existing_trimmed)
````

### src-tauri/src/session/store.rs:575-630

````rust
  575:     fn create_latest_schema(conn: &Connection) -> rusqlite::Result<()> {
  576:         conn.execute_batch(
  577:             "CREATE TABLE IF NOT EXISTS sessions (
  578:                 id TEXT PRIMARY KEY,
  579:                 title TEXT NOT NULL,
  580:                 parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  581:                 workspace_id TEXT,
  582:                 session_type TEXT NOT NULL DEFAULT 'chat',
  583:                 agent_id TEXT,
  584:                 archived_at INTEGER,
  585:                 latest_completed_run_id TEXT,
  586:                 latest_todo_run_id TEXT,
  587:                 created_at INTEGER NOT NULL,
  588:                 updated_at INTEGER NOT NULL
  589:             );
  590:             CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
  591:             CREATE INDEX IF NOT EXISTS idx_sessions_workspace ON sessions(workspace_id);
  592: 
  593:             CREATE TABLE IF NOT EXISTS messages (
  594:                 id TEXT PRIMARY KEY,
  595:                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  596:                 role TEXT NOT NULL,
  597:                 content TEXT NOT NULL,
  598:                 created_at INTEGER NOT NULL,
  599:                 prompt_prefix TEXT,
  600:                 prompt_suffix TEXT,
  601:                 tool_calls TEXT,
  602:                 tool_call_id TEXT,
  603:                 images TEXT,
  604:                 asset_refs TEXT,
  605:                 thinking_content TEXT,
  606:                 thinking_duration INTEGER,
  607:                 thinking_signature TEXT,
  608:                 metadata_json TEXT,
  609:                 include_in_prompt INTEGER NOT NULL DEFAULT 1
  610:             );
  611:             CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
  612: 
  613:             CREATE TABLE IF NOT EXISTS token_usage (
  614:                 session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  615:                 total_input_tokens INTEGER NOT NULL DEFAULT 0,
  616:                 total_output_tokens INTEGER NOT NULL DEFAULT 0,
  617:                 total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  618:                 total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  619:                 total_cost_usd REAL NOT NULL DEFAULT 0,
  620:                 priced_rounds INTEGER NOT NULL DEFAULT 0,
  621:                 last_context_tokens INTEGER NOT NULL DEFAULT 0,
  622:                 last_context_limit INTEGER NOT NULL DEFAULT 0
  623:             );
  624: 
  625:             CREATE TABLE IF NOT EXISTS todos (
  626:                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  627:                 position INTEGER NOT NULL,
  628:                 content TEXT NOT NULL,
  629:                 status TEXT NOT NULL DEFAULT 'pending',
  630:                 priority TEXT NOT NULL DEFAULT 'medium',
````

### src-tauri/src/session/store.rs:842-970

````rust
  842:     fn migrate_message_render_orders(conn: &Connection) -> rusqlite::Result<()> {
  843:         fn to_sql_error(
  844:             error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
  845:         ) -> rusqlite::Error {
  846:             rusqlite::Error::ToSqlConversionFailure(error.into())
  847:         }
  848: 
  849:         fn bump_next_order(next_order: &mut u32, order: Option<u32>) {
  850:             if let Some(order) = order.filter(|value| *value > 0) {
  851:                 *next_order = (*next_order).max(order.saturating_add(1));
  852:             }
  853:         }
  854: 
  855:         fn assign_tool_call_orders(tool_calls: &mut [ToolCallInfo], next_order: &mut u32) -> bool {
  856:             let mut changed = false;
  857:             for tool_call in tool_calls {
  858:                 if tool_call.order.is_none() {
  859:                     tool_call.order = Some(*next_order);
  860:                     *next_order = next_order.saturating_add(1);
  861:                     changed = true;
  862:                 } else {
  863:                     bump_next_order(next_order, tool_call.order);
  864:                 }
  865: 
  866:                 if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_mut() {
  867:                     changed |= assign_tool_call_orders(nested_tool_calls, next_order);
  868:                 }
  869:             }
  870:             changed
  871:         }
  872: 
  873:         let mut stmt = conn.prepare(
  874:             "SELECT id, role, content, tool_calls, thinking_content, metadata_json
  875:              FROM messages
  876:              ORDER BY created_at ASC, rowid ASC",
  877:         )?;
  878:         let rows = stmt
  879:             .query_map([], |row| {
  880:                 Ok((
  881:                     row.get::<_, String>(0)?,
  882:                     row.get::<_, String>(1)?,
  883:                     row.get::<_, String>(2)?,
  884:                     row.get::<_, Option<String>>(3)?,
  885:                     row.get::<_, Option<String>>(4)?,
  886:                     row.get::<_, Option<String>>(5)?,
  887:                 ))
  888:             })?
  889:             .collect::<Result<Vec<_>, _>>()?;
  890:         drop(stmt);
  891: 
  892:         for (message_id, role, content, tool_calls_json, thinking_content, metadata_json) in rows {
  893:             if role != "assistant" {
  894:                 continue;
  895:             }
  896: 
  897:             let mut metadata: MessageMetadata = metadata_json
  898:                 .as_deref()
  899:                 .map(serde_json::from_str)
  900:                 .transpose()
  901:                 .map_err(to_sql_error)?
  902:                 .unwrap_or_default();
  903:             let mut metadata_changed = false;
  904:             let mut next_order = 1u32;
  905: 
  906:             bump_next_order(&mut next_order, metadata.thinking_order);
  907:             bump_next_order(&mut next_order, metadata.content_order);
  908: 
  909:             let has_thinking = thinking_content
  910:                 .as_deref()
  911:                 .map(str::trim)
  912:                 .is_some_and(|value| !value.is_empty());
  913:             if has_thinking && metadata.thinking_order.is_none() {
  914:                 metadata.thinking_order = Some(next_order);
  915:                 next_order = next_order.saturating_add(1);
  916:                 metadata_changed = true;
  917:             }
  918: 
  919:             if !content.trim().is_empty() && metadata.content_order.is_none() {
  920:                 metadata.content_order = Some(next_order);
  921:                 next_order = next_order.saturating_add(1);
  922:                 metadata_changed = true;
  923:             }
  924: 
  925:             let mut next_tool_calls_json = None;
  926:             if let Some(tool_calls_json) = tool_calls_json.as_deref() {
  927:                 let mut tool_calls: Vec<ToolCallInfo> =
  928:                     serde_json::from_str(tool_calls_json).map_err(to_sql_error)?;
  929:                 if assign_tool_call_orders(&mut tool_calls, &mut next_order) {
  930:                     next_tool_calls_json =
  931:                         Some(serde_json::to_string(&tool_calls).map_err(to_sql_error)?);
  932:                 }
  933:             }
  934: 
  935:             let next_metadata_json = if metadata_changed {
  936:                 if metadata.knowledge_proposal.is_none()
  937:                     && metadata.response_id.is_none()
  938:                     && metadata.response_request.is_none()
  939:                     && metadata.content_order.is_none()
  940:                     && metadata.thinking_order.is_none()
  941:                     && metadata.render_parts.is_none()
  942:                 {
  943:                     Some(None)
  944:                 } else {
  945:                     Some(Some(
  946:                         serde_json::to_string(&metadata).map_err(to_sql_error)?,
  947:                     ))
  948:                 }
  949:             } else {
  950:                 None
  951:             };
  952: 
  953:             if next_metadata_json.is_none() && next_tool_calls_json.is_none() {
  954:                 continue;
  955:             }
  956: 
  957:             conn.execute(
  958:                 "UPDATE messages
  959:                  SET metadata_json = COALESCE(?1, metadata_json),
  960:                      tool_calls = COALESCE(?2, tool_calls)
  961:                  WHERE id = ?3",
  962:                 params![
  963:                     next_metadata_json.flatten(),
  964:                     next_tool_calls_json,
  965:                     message_id,
  966:                 ],
  967:             )?;
  968:         }
  969: 
  970:         Ok(())
````

### src-tauri/src/session/store.rs:1686-1950

````rust
 1686:     pub fn add_message_with_thinking(
 1687:         &self,
 1688:         session_id: &str,
 1689:         role: MessageRole,
 1690:         content: &str,
 1691:         thinking_content: Option<&str>,
 1692:         thinking_duration: Option<u32>,
 1693:         thinking_signature: Option<&str>,
 1694:         response_id: Option<&str>,
 1695:         response_request: Option<&serde_json::Value>,
 1696:     ) -> Result<String, String> {
 1697:         self.add_message_with_thinking_and_order(
 1698:             session_id,
 1699:             role,
 1700:             content,
 1701:             thinking_content,
 1702:             thinking_duration,
 1703:             thinking_signature,
 1704:             response_id,
 1705:             response_request,
 1706:             None,
 1707:             None,
 1708:         )
 1709:     }
 1710: 
 1711:     pub fn add_message_with_thinking_and_order(
 1712:         &self,
 1713:         session_id: &str,
 1714:         role: MessageRole,
 1715:         content: &str,
 1716:         thinking_content: Option<&str>,
 1717:         thinking_duration: Option<u32>,
 1718:         thinking_signature: Option<&str>,
 1719:         response_id: Option<&str>,
 1720:         response_request: Option<&serde_json::Value>,
 1721:         content_order: Option<u32>,
 1722:         thinking_order: Option<u32>,
 1723:     ) -> Result<String, String> {
 1724:         self.add_message_full_with_thinking(
 1725:             session_id,
 1726:             role,
 1727:             content,
 1728:             None,
 1729:             None,
 1730:             None,
 1731:             None,
 1732:             thinking_content,
 1733:             thinking_duration,
 1734:             thinking_signature,
 1735:             None,
 1736:             None,
 1737:             None,
 1738:             response_id,
 1739:             response_request,
 1740:             content_order,
 1741:             thinking_order,
 1742:         )
 1743:     }
 1744: 
 1745:     pub fn add_message_with_thinking_and_render_parts(
 1746:         &self,
 1747:         session_id: &str,
 1748:         role: MessageRole,
 1749:         content: &str,
 1750:         thinking_content: Option<&str>,
 1751:         thinking_duration: Option<u32>,
 1752:         thinking_signature: Option<&str>,
 1753:         response_id: Option<&str>,
 1754:         response_request: Option<&serde_json::Value>,
 1755:         content_order: Option<u32>,
 1756:         thinking_order: Option<u32>,
 1757:         render_parts: &[AssistantRenderPart],
 1758:     ) -> Result<String, String> {
 1759:         self.add_message_full_with_thinking_and_render_parts(
 1760:             session_id,
 1761:             role,
 1762:             content,
 1763:             None,
 1764:             None,
 1765:             None,
 1766:             None,
 1767:             thinking_content,
 1768:             thinking_duration,
 1769:             thinking_signature,
 1770:             None,
 1771:             None,
 1772:             None,
 1773:             response_id,
 1774:             response_request,
 1775:             content_order,
 1776:             thinking_order,
 1777:             Some(render_parts),
 1778:         )
 1779:     }
 1780: 
 1781:     #[allow(dead_code)]
 1782:     pub fn add_assistant_with_tool_calls(
 1783:         &self,
 1784:         session_id: &str,
 1785:         content: &str,
 1786:         tool_calls: &[ToolCallInfo],
 1787:     ) -> Result<String, String> {
 1788:         self.add_assistant_with_tool_calls_and_thinking(
 1789:             session_id, content, tool_calls, None, None, None, None, None,
 1790:         )
 1791:     }
 1792: 
 1793:     pub fn add_assistant_with_tool_calls_and_thinking(
 1794:         &self,
 1795:         session_id: &str,
 1796:         content: &str,
 1797:         tool_calls: &[ToolCallInfo],
 1798:         thinking_content: Option<&str>,
 1799:         thinking_duration: Option<u32>,
 1800:         thinking_signature: Option<&str>,
 1801:         response_id: Option<&str>,
 1802:         response_request: Option<&serde_json::Value>,
 1803:     ) -> Result<String, String> {
 1804:         self.add_assistant_with_tool_calls_and_thinking_and_order(
 1805:             session_id,
 1806:             content,
 1807:             tool_calls,
 1808:             thinking_content,
 1809:             thinking_duration,
 1810:             thinking_signature,
 1811:             response_id,
 1812:             response_request,
 1813:             None,
 1814:             None,
 1815:         )
 1816:     }
 1817: 
 1818:     pub fn add_assistant_with_tool_calls_and_thinking_and_order(
 1819:         &self,
 1820:         session_id: &str,
 1821:         content: &str,
 1822:         tool_calls: &[ToolCallInfo],
 1823:         thinking_content: Option<&str>,
 1824:         thinking_duration: Option<u32>,
 1825:         thinking_signature: Option<&str>,
 1826:         response_id: Option<&str>,
 1827:         response_request: Option<&serde_json::Value>,
 1828:         content_order: Option<u32>,
 1829:         thinking_order: Option<u32>,
 1830:     ) -> Result<String, String> {
 1831:         let tool_calls_json = serde_json::to_string(tool_calls)
 1832:             .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
 1833:         self.add_message_full_with_thinking(
 1834:             session_id,
 1835:             MessageRole::Assistant,
 1836:             content,
 1837:             Some(&tool_calls_json),
 1838:             None,
 1839:             None,
 1840:             None,
 1841:             thinking_content,
 1842:             thinking_duration,
 1843:             thinking_signature,
 1844:             None,
 1845:             None,
 1846:             None,
 1847:             response_id,
 1848:             response_request,
 1849:             content_order,
 1850:             thinking_order,
 1851:         )
 1852:     }
 1853: 
 1854:     pub fn add_assistant_with_tool_calls_and_render_parts(
 1855:         &self,
 1856:         session_id: &str,
 1857:         content: &str,
 1858:         tool_calls: &[ToolCallInfo],
 1859:         thinking_content: Option<&str>,
 1860:         thinking_duration: Option<u32>,
 1861:         thinking_signature: Option<&str>,
 1862:         response_id: Option<&str>,
 1863:         response_request: Option<&serde_json::Value>,
 1864:         content_order: Option<u32>,
 1865:         thinking_order: Option<u32>,
 1866:         render_parts: &[AssistantRenderPart],
 1867:     ) -> Result<String, String> {
 1868:         let tool_calls_json = serde_json::to_string(tool_calls)
 1869:             .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
 1870:         self.add_message_full_with_thinking_and_render_parts(
 1871:             session_id,
 1872:             MessageRole::Assistant,
 1873:             content,
 1874:             Some(&tool_calls_json),
 1875:             None,
 1876:             None,
 1877:             None,
 1878:             thinking_content,
 1879:             thinking_duration,
 1880:             thinking_signature,
 1881:             None,
 1882:             None,
 1883:             None,
 1884:             response_id,
 1885:             response_request,
 1886:             content_order,
 1887:             thinking_order,
 1888:             Some(render_parts),
 1889:         )
 1890:     }
 1891: 
 1892:     pub fn update_message_tool_calls(
 1893:         &self,
 1894:         message_id: &str,
 1895:         tool_calls: &[ToolCallInfo],
 1896:     ) -> Result<(), String> {
 1897:         let tool_calls_json = serde_json::to_string(tool_calls)
 1898:             .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
 1899:         let conn = self.conn.lock().map_err(|e| e.to_string())?;
 1900:         conn.execute(
 1901:             "UPDATE messages SET tool_calls = ?1 WHERE id = ?2",
 1902:             params![tool_calls_json, message_id],
 1903:         )
 1904:         .map_err(|e| {
 1905:             format!(
 1906:                 "Failed to update tool_calls for message '{}': {}",
 1907:                 message_id, e
 1908:             )
 1909:         })?;
 1910:         Ok(())
 1911:     }
 1912: 
 1913:     pub fn update_message_tool_calls_and_render_parts(
 1914:         &self,
 1915:         message_id: &str,
 1916:         tool_calls: &[ToolCallInfo],
 1917:         render_parts: &[AssistantRenderPart],
 1918:     ) -> Result<(), String> {
 1919:         let tool_calls_json = serde_json::to_string(tool_calls)
 1920:             .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
 1921:         let render_parts = render_parts.to_vec();
 1922:         let conn = self.conn.lock().map_err(|e| e.to_string())?;
 1923:         let metadata_json: Option<String> = conn
 1924:             .query_row(
 1925:                 "SELECT metadata_json FROM messages WHERE id = ?1",
 1926:                 params![message_id],
 1927:                 |row| row.get(0),
 1928:             )
 1929:             .optional()
 1930:             .map_err(|e| format!("Failed to load message metadata: {}", e))?
 1931:             .flatten();
 1932:         let mut metadata: MessageMetadata = metadata_json
 1933:             .as_deref()
 1934:             .map(serde_json::from_str)
 1935:             .transpose()
 1936:             .map_err(|e| format!("Failed to parse message metadata: {}", e))?
 1937:             .unwrap_or_default();
 1938:         metadata.render_parts = Some(render_parts);
 1939:         let metadata_json = serde_json::to_string(&metadata)
 1940:             .map_err(|e| format!("Failed to serialize message metadata: {}", e))?;
 1941:         conn.execute(
 1942:             "UPDATE messages SET tool_calls = ?1, metadata_json = ?2 WHERE id = ?3",
 1943:             params![tool_calls_json, metadata_json, message_id],
 1944:         )
 1945:         .map_err(|e| {
 1946:             format!(
 1947:                 "Failed to update tool_calls/render_parts for message '{}': {}",
 1948:                 message_id, e
 1949:             )
 1950:         })?;
````

### src-tauri/src/session/store.rs:2140-2220

````rust
 2140:         prompt_prefix: Option<&str>,
 2141:         prompt_suffix: Option<&str>,
 2142:         response_id: Option<&str>,
 2143:         response_request: Option<&serde_json::Value>,
 2144:         content_order: Option<u32>,
 2145:         thinking_order: Option<u32>,
 2146:         render_parts: Option<&[AssistantRenderPart]>,
 2147:     ) -> Result<String, String> {
 2148:         let id = Uuid::new_v4().to_string();
 2149:         let now = Self::now_ts();
 2150:         let metadata_json = message_metadata_json(
 2151:             knowledge_proposal,
 2152:             response_id,
 2153:             response_request,
 2154:             content_order,
 2155:             thinking_order,
 2156:             render_parts,
 2157:         )?;
 2158:         let conn = self.conn.lock().map_err(|e| e.to_string())?;
 2159: 
 2160:         conn.execute(
 2161:             "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, asset_refs, thinking_content, thinking_duration, thinking_signature, metadata_json)
 2162:              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
 2163:             params![id, session_id, role.as_str(), content, now, prompt_prefix, prompt_suffix, tool_calls_json, tool_call_id, images_json, asset_refs_json, thinking_content, thinking_duration.map(|d| d as i64), thinking_signature, metadata_json],
 2164:         )
 2165:         .map_err(|e| format!("Failed to add message: {}", e))?;
 2166: 
 2167:         conn.execute(
 2168:             "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
 2169:             params![now, session_id],
 2170:         )
 2171:         .map_err(|e| format!("Failed to update session: {}", e))?;
 2172: 
 2173:         Ok(id)
 2174:     }
 2175: 
 2176:     pub fn add_knowledge_proposal_message(
 2177:         &self,
 2178:         session_id: &str,
 2179:         proposal: &KnowledgeProposal,
 2180:     ) -> Result<String, String> {
 2181:         self.add_message_full(
 2182:             session_id,
 2183:             MessageRole::Assistant,
 2184:             "",
 2185:             None,
 2186:             None,
 2187:             None,
 2188:             None,
 2189:             None,
 2190:             Some(proposal),
 2191:         )
 2192:     }
 2193: 
 2194:     pub fn get_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
 2195:         let conn = self.conn.lock().map_err(|e| e.to_string())?;
 2196:         self.get_messages_with_conn_filtered(&conn, session_id, false)
 2197:     }
 2198: 
 2199:     pub fn get_messages_for_prompt(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
 2200:         let conn = self.conn.lock().map_err(|e| e.to_string())?;
 2201:         self.get_messages_with_conn_filtered(&conn, session_id, true)
 2202:     }
 2203: 
 2204:     pub fn get_response_request_metadata(
 2205:         &self,
 2206:         session_id: &str,
 2207:     ) -> Result<HashMap<String, serde_json::Value>, String> {
 2208:         let conn = self.conn.lock().map_err(|e| e.to_string())?;
 2209:         let mut stmt = conn
 2210:             .prepare(
 2211:                 "SELECT id, metadata_json FROM messages
 2212:                  WHERE session_id = ?1 AND metadata_json IS NOT NULL
 2213:                  ORDER BY created_at ASC, rowid ASC",
 2214:             )
 2215:             .map_err(|e| format!("Failed to prepare response request query: {}", e))?;
 2216: 
 2217:         let rows = stmt
 2218:             .query_map(params![session_id], |row| {
 2219:                 Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
 2220:             })
````

### src-tauri/src/session/store.rs:2700-2795

````rust
 2700:             "NULL AS asset_refs"
 2701:         };
 2702:         let query = if prompt_only {
 2703:             format!(
 2704:                 "SELECT id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, {asset_refs_select}, thinking_content, thinking_duration, thinking_signature, metadata_json
 2705:              FROM messages
 2706:              WHERE session_id = ?1 AND include_in_prompt = 1
 2707:              ORDER BY created_at ASC, rowid ASC"
 2708:             )
 2709:         } else {
 2710:             format!(
 2711:                 "SELECT id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, {asset_refs_select}, thinking_content, thinking_duration, thinking_signature, metadata_json
 2712:              FROM messages
 2713:              WHERE session_id = ?1
 2714:              ORDER BY rowid ASC"
 2715:             )
 2716:         };
 2717: 
 2718:         let mut stmt = conn
 2719:             .prepare(&query)
 2720:             .map_err(|e| format!("Failed to prepare query: {}", e))?;
 2721: 
 2722:         let rows = stmt
 2723:             .query_map(params![session_id], |row| {
 2724:                 Ok((
 2725:                     row.get::<_, String>(0)?,
 2726:                     row.get::<_, String>(1)?,
 2727:                     row.get::<_, String>(2)?,
 2728:                     row.get::<_, i64>(3)?,
 2729:                     row.get::<_, Option<String>>(4)?,
 2730:                     row.get::<_, Option<String>>(5)?,
 2731:                     row.get::<_, Option<String>>(6)?,
 2732:                     row.get::<_, Option<String>>(7)?,
 2733:                     row.get::<_, Option<String>>(8)?,
 2734:                     row.get::<_, Option<String>>(9)?,
 2735:                     row.get::<_, Option<String>>(10)?,
 2736:                     row.get::<_, Option<i64>>(11)?,
 2737:                     row.get::<_, Option<String>>(12)?,
 2738:                     row.get::<_, Option<String>>(13)?,
 2739:                 ))
 2740:             })
 2741:             .map_err(|e| format!("Failed to query messages: {}", e))?;
 2742: 
 2743:         let mut messages = Vec::new();
 2744:         for row in rows {
 2745:             let (
 2746:                 id,
 2747:                 role_str,
 2748:                 content,
 2749:                 created_at,
 2750:                 prompt_prefix,
 2751:                 prompt_suffix,
 2752:                 tool_calls_json,
 2753:                 tool_call_id,
 2754:                 images_json,
 2755:                 asset_refs_json,
 2756:                 thinking_content,
 2757:                 thinking_duration_raw,
 2758:                 thinking_signature,
 2759:                 metadata_json,
 2760:             ) = row.map_err(|e| format!("Failed to read row: {}", e))?;
 2761:             let role = MessageRole::from_str(&role_str)?;
 2762: 
 2763:             let tool_calls: Option<Vec<ToolCallInfo>> = tool_calls_json
 2764:                 .as_deref()
 2765:                 .map(|json| serde_json::from_str(json))
 2766:                 .transpose()
 2767:                 .map_err(|e| format!("Failed to parse tool_calls: {}", e))?;
 2768: 
 2769:             let images: Option<Vec<super::models::ImageData>> = images_json
 2770:                 .as_deref()
 2771:                 .map(|json| serde_json::from_str(json))
 2772:                 .transpose()
 2773:                 .map_err(|e| format!("Failed to parse images: {}", e))?;
 2774: 
 2775:             let asset_refs: Option<Vec<super::models::AssetRefData>> = asset_refs_json
 2776:                 .as_deref()
 2777:                 .map(|json| serde_json::from_str(json))
 2778:                 .transpose()
 2779:                 .map_err(|e| format!("Failed to parse asset refs: {}", e))?;
 2780: 
 2781:             let metadata: Option<MessageMetadata> = metadata_json
 2782:                 .as_deref()
 2783:                 .map(|json| serde_json::from_str(json))
 2784:                 .transpose()
 2785:                 .map_err(|e| format!("Failed to parse message metadata: {}", e))?;
 2786:             let (knowledge_proposal, response_id, content_order, thinking_order, render_parts) =
 2787:                 metadata
 2788:                     .map(|value| {
 2789:                         (
 2790:                             value.knowledge_proposal,
 2791:                             value.response_id,
 2792:                             value.content_order,
 2793:                             value.thinking_order,
 2794:                             value.render_parts,
 2795:                         )
````

### src-tauri/src/agent/instance/mod.rs:146-270

````rust
  146: struct StreamRenderOrderTracker {
  147:     next_seq: u32,
  148:     part_orders: HashMap<String, u32>,
  149: }
  150: 
  151: #[derive(Debug, Clone)]
  152: struct RenderPartMark {
  153:     id: String,
  154:     seq: u32,
  155: }
  156: 
  157: impl StreamRenderOrderTracker {
  158:     fn next(&mut self) -> u32 {
  159:         self.next_seq = self.next_seq.saturating_add(1).max(1);
  160:         self.next_seq
  161:     }
  162: 
  163:     fn mark_part(&mut self, run_id: &str, stable_key: &str) -> RenderPartMark {
  164:         if let Some(seq) = self.part_orders.get(stable_key).copied() {
  165:             return RenderPartMark {
  166:                 id: format!("{}:{}", run_id, stable_key),
  167:                 seq,
  168:             };
  169:         }
  170:         let seq = self.next();
  171:         self.part_orders.insert(stable_key.to_string(), seq);
  172:         RenderPartMark {
  173:             id: format!("{}:{}", run_id, stable_key),
  174:             seq,
  175:         }
  176:     }
  177: 
  178:     fn mark_text(&mut self, run_id: &str, block_id: &str) -> RenderPartMark {
  179:         self.mark_part(run_id, &format!("text:{}", block_id))
  180:     }
  181: 
  182:     fn mark_thinking(&mut self, run_id: &str, block_id: &str) -> RenderPartMark {
  183:         self.mark_part(run_id, &format!("thinking:{}", block_id))
  184:     }
  185: 
  186:     fn mark_tool(&mut self, run_id: &str, tool_call_id: &str) -> RenderPartMark {
  187:         self.mark_part(run_id, &format!("tool:{}", tool_call_id))
  188:     }
  189: 
  190:     fn assign_tool_orders_for_run(
  191:         &mut self,
  192:         run_id: &str,
  193:         tool_calls: &[ToolCallInfo],
  194:     ) -> Vec<ToolCallInfo> {
  195:         tool_calls
  196:             .iter()
  197:             .map(|tool_call| self.assign_tool_order(run_id, tool_call))
  198:             .collect()
  199:     }
  200: 
  201:     fn assign_tool_order(&mut self, run_id: &str, tool_call: &ToolCallInfo) -> ToolCallInfo {
  202:         let mut tool_call = tool_call.clone();
  203:         if tool_call.order.is_none() {
  204:             let mark = self.mark_tool(run_id, &tool_call.id);
  205:             tool_call.order = Some(mark.seq);
  206:         }
  207:         if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_ref() {
  208:             tool_call.nested_tool_calls = Some(
  209:                 nested_tool_calls
  210:                     .iter()
  211:                     .map(|nested| self.assign_tool_order(run_id, nested))
  212:                     .collect(),
  213:             );
  214:         }
  215:         tool_call
  216:     }
  217: }
  218: 
  219: fn render_order_key(run_id: &str, seq: u32) -> RenderOrderKey {
  220:     RenderOrderKey {
  221:         run_id: run_id.to_string(),
  222:         seq,
  223:     }
  224: }
  225: 
  226: fn assistant_render_parts_for_response(
  227:     run_id: &str,
  228:     text_part: Option<RenderPartMark>,
  229:     text: &str,
  230:     thinking_part: Option<RenderPartMark>,
  231:     thinking_text: &str,
  232:     thinking_duration: Option<u32>,
  233:     thinking_signature: Option<&str>,
  234:     tool_calls: &[ToolCallInfo],
  235: ) -> Vec<AssistantRenderPart> {
  236:     let mut parts = Vec::new();
  237:     if let Some(mark) = thinking_part.filter(|_| !thinking_text.is_empty()) {
  238:         parts.push(AssistantRenderPart::Thinking {
  239:             id: mark.id,
  240:             order: render_order_key(run_id, mark.seq),
  241:             content: thinking_text.to_string(),
  242:             active: Some(false),
  243:             duration: thinking_duration,
  244:             signature: thinking_signature
  245:                 .filter(|value| !value.is_empty())
  246:                 .map(str::to_string),
  247:         });
  248:     }
  249:     if let Some(mark) = text_part.filter(|_| !text.is_empty()) {
  250:         parts.push(AssistantRenderPart::Text {
  251:             id: mark.id,
  252:             order: render_order_key(run_id, mark.seq),
  253:             content: text.to_string(),
  254:         });
  255:     }
  256:     for tool_call in tool_calls {
  257:         if let Some(seq) = tool_call.order {
  258:             parts.push(AssistantRenderPart::ToolCall {
  259:                 id: tool_call.id.clone(),
  260:                 order: render_order_key(run_id, seq),
  261:                 tool_call: tool_call.clone(),
  262:             });
  263:         }
  264:     }
  265:     parts.sort_by(|left, right| render_part_seq(left).cmp(&render_part_seq(right)));
  266:     parts
  267: }
  268: 
  269: fn render_part_seq(part: &AssistantRenderPart) -> u32 {
  270:     match part {
````

### src-tauri/src/agent/instance/mod.rs:5560-5855

````rust
 5560:         let final_continuation_request;
 5561:         let final_content_order;
 5562:         let final_thinking_order;
 5563:         let mut done_already_emitted = false;
 5564:         let mut terminal_done_message_id: Option<String> = None;
 5565:         let mut codex_turn_state = matches!(self.backend, LlmBackend::OpenAiCodex { .. })
 5566:             .then(codex::TurnState::default);
 5567:         let render_order_tracker = Arc::new(Mutex::new(StreamRenderOrderTracker::default()));
 5568: 
 5569:         'agent_loop: loop {
 5570:             iteration += 1;
 5571:             if iteration > MAX_TOOL_ITERATIONS {
 5572:                 return Err(format!(
 5573:                     "Agent loop exceeded max iterations ({})",
 5574:                     MAX_TOOL_ITERATIONS
 5575:                 ));
 5576:             }
 5577: 
 5578:             if self.is_cancel_requested() {
 5579:                 self.clear_pending_knowledge_proposal(app_handle).await;
 5580:                 self.emit_cancelled(app_handle, store, &run_id);
 5581:                 return Ok(String::new());
 5582:             }
 5583: 
 5584:             let messages = store.get_messages_for_prompt(&self.session_id)?;
 5585: 
 5586:             let session_id = self.session_id.clone();
 5587:             let handle = app_handle.clone();
 5588:             let parent_tc = self.parent_tool_call.clone();
 5589:             let system_parts: Vec<&str> = {
 5590:                 let mut parts = vec![prompt_parts.base_prompt.as_str()];
 5591:                 if !prompt_parts.rules_prompt.is_empty() {
 5592:                     parts.push(prompt_parts.rules_prompt.as_str());
 5593:                 }
 5594:                 if !prompt_parts.knowledge_prompt.is_empty() {
 5595:                     parts.push(prompt_parts.knowledge_prompt.as_str());
 5596:                 }
 5597:                 parts
 5598:             };
 5599:             let ctx_limit = if let LlmBackend::Custom { context_length, .. } = &self.backend {
 5600:                 *context_length
 5601:             } else {
 5602:                 model_context_limit(&self.effective_model)
 5603:             };
 5604:             let prepared_messages = compact::prepare_messages_for_llm(&messages);
 5605:             let estimated_input_tokens =
 5606:                 compact::estimate_request_tokens(&system_parts, &prepared_messages, &api_tools);
 5607:             let is_codex_backend = matches!(self.backend, LlmBackend::OpenAiCodex { .. });
 5608:             let should_preflight_compact = if is_codex_backend {
 5609:                 compact::should_codex_auto_compact(estimated_input_tokens, ctx_limit)
 5610:             } else {
 5611:                 compact::should_auto_compact(estimated_input_tokens, ctx_limit)
 5612:             };
 5613:             let mut preflight_compact_error: Option<String> = None;
 5614: 
 5615:             if !compact_tracker.is_circuit_broken()
 5616:                 && should_preflight_compact
 5617:             {
 5618:                 eprintln!(
 5619:                     "[Agent {}] preflight auto-compact candidate: estimated_tokens={}, limit={}, messages={} -> {}",
 5620:                     self.id,
 5621:                     estimated_input_tokens,
 5622:                     ctx_limit,
 5623:                     messages.len(),
 5624:                     prepared_messages.len()
 5625:                 );
 5626:                 match self
 5627:                     .execute_auto_compact(
 5628:                         app_handle,
 5629:                         store,
 5630:                         &system_parts,
 5631:                         estimated_input_tokens,
 5632:                         ctx_limit,
 5633:                         false,
 5634:                         &run_id,
 5635:                         "compact",
 5636:                         iteration,
 5637:                     )
 5638:                     .await
 5639:                 {
 5640:                     Ok(true) => {
 5641:                         compact_tracker.record_success();
 5642:                         eprintln!("[Agent {}] preflight auto-compact succeeded", self.id);
 5643:                         continue 'agent_loop;
 5644:                     }
 5645:                     Ok(false) => {}
 5646:                     Err(e) => {
 5647:                         compact_tracker.record_failure();
 5648:                         eprintln!("[Agent {}] preflight auto-compact failed: {}", self.id, e);
 5649:                         preflight_compact_error = Some(e);
 5650:                     }
 5651:                 }
 5652:             }
 5653: 
 5654:             if is_codex_backend
 5655:                 && compact::should_codex_block_normal_send(estimated_input_tokens, ctx_limit)
 5656:             {
 5657:                 let reason = preflight_compact_error
 5658:                     .unwrap_or_else(|| "Codex request is too close to the context limit".to_string());
 5659:                 return Err(format!(
 5660:                     "Refusing to send oversized Codex request after compact failed or was unavailable: estimated_input_tokens={}, limit={}, reason={}",
 5661:                     estimated_input_tokens, ctx_limit, reason
 5662:                 ));
 5663:             }
 5664: 
 5665:             eprintln!(
 5666:                 "[Agent {}] iteration {}, messages={}, prepared_messages={}, estimated_input_tokens={}",
 5667:                 self.id,
 5668:                 iteration,
 5669:                 messages.len(),
 5670:                 prepared_messages.len(),
 5671:                 estimated_input_tokens
 5672:             );
 5673: 
 5674:             const LLM_RETRIES: u32 = 2;
 5675:             let mut response = None;
 5676:             let mut response_text_part: Option<RenderPartMark> = None;
 5677:             let mut response_thinking_part: Option<RenderPartMark> = None;
 5678:             let mut last_llm_error = String::new();
 5679:             let mut needs_reactive_compact = false;
 5680: 
 5681:             for llm_attempt in 0..=LLM_RETRIES {
 5682:                 let attempt_number = llm_attempt + 1;
 5683:                 let llm_call_started_at = Instant::now();
 5684:                 eprintln!(
 5685:                     "[Agent {}] LLM attempt start: session={} run={} iteration={} attempt={}/{} backend={} prepared_messages={} api_tools={} estimated_input_tokens={}",
 5686:                     self.id,
 5687:                     self.session_id,
 5688:                     run_id,
 5689:                     iteration,
 5690:                     attempt_number,
 5691:                     LLM_RETRIES + 1,
 5692:                     backend_name,
 5693:                     prepared_messages.len(),
 5694:                     api_tools.len(),
 5695:                     estimated_input_tokens
 5696:                 );
 5697:                 let sid = session_id.clone();
 5698:                 let hdl = handle.clone();
 5699:                 let ptc = parent_tc.clone();
 5700:                 let rid = run_id.clone();
 5701:                 let render_order_for_text = render_order_tracker.clone();
 5702:                 let text_block_id = format!("iteration:{}:attempt:{}:text", iteration, attempt_number);
 5703:                 let partial_for_text = self.partial_assistant.clone();
 5704:                 let agent_id_for_text = self.id.clone();
 5705:                 let first_text_delta_logged = Arc::new(AtomicBool::new(false));
 5706:                 let first_text_delta_logged_for_cb = first_text_delta_logged.clone();
 5707:                 let attempt_emitted_output = Arc::new(AtomicBool::new(false));
 5708:                 let emitted_output_for_text = attempt_emitted_output.clone();
 5709: 
 5710:                 let sid2 = session_id.clone();
 5711:                 let hdl2 = handle.clone();
 5712:                 let rid2 = run_id.clone();
 5713:                 let render_order_for_thinking = render_order_tracker.clone();
 5714:                 let thinking_block_id =
 5715:                     format!("iteration:{}:attempt:{}:thinking", iteration, attempt_number);
 5716:                 let partial_for_thinking = self.partial_assistant.clone();
 5717:                 let agent_id_for_thinking = self.id.clone();
 5718:                 let first_thinking_delta_logged = Arc::new(AtomicBool::new(false));
 5719:                 let first_thinking_delta_logged_for_cb = first_thinking_delta_logged.clone();
 5720:                 let emitted_output_for_thinking = attempt_emitted_output.clone();
 5721: 
 5722:                 let sid3 = session_id.clone();
 5723:                 let hdl3 = handle.clone();
 5724:                 let ptc3 = parent_tc.clone();
 5725:                 let rid3 = run_id.clone();
 5726:                 let render_order_for_tool = render_order_tracker.clone();
 5727:                 let agent_id_for_tool_start = self.id.clone();
 5728:                 let first_tool_call_logged = Arc::new(AtomicBool::new(false));
 5729:                 let first_tool_call_logged_for_cb = first_tool_call_logged.clone();
 5730:                 let emitted_output_for_tool = attempt_emitted_output.clone();
 5731:                 let tool_registry_for_tool_start = self.tool_registry.clone();
 5732: 
 5733:                 let mut cancel_rx = self.cancel_waiter();
 5734:                 let result = tokio::select! {
 5735:                     result = self.call_llm(
 5736:                         store,
 5737:                         codex_turn_state.as_mut(),
 5738:                         &system_parts,
 5739:                         &prepared_messages,
 5740:                         &api_tools,
 5741:                         move |delta| {
 5742:                             emitted_output_for_text.store(true, Ordering::Relaxed);
 5743:                             let mark = render_order_for_text
 5744:                                 .lock()
 5745:                                 .map(|mut tracker| tracker.mark_text(&rid, &text_block_id))
 5746:                                 .unwrap_or(RenderPartMark {
 5747:                                     id: format!("{}:text:{}", rid, text_block_id),
 5748:                                     seq: 1,
 5749:                                 });
 5750:                             if !first_text_delta_logged_for_cb.swap(true, Ordering::Relaxed) {
 5751:                                 eprintln!(
 5752:                                     "[Agent {}] first text delta: session={} run={} iteration={} attempt={}/{} elapsed_ms={} delta_len={}",
 5753:                                     agent_id_for_text,
 5754:                                     sid,
 5755:                                     rid,
 5756:                                     iteration,
 5757:                                     attempt_number,
 5758:                                     LLM_RETRIES + 1,
 5759:                                     llm_call_started_at.elapsed().as_millis(),
 5760:                                     delta.len()
 5761:                                 );
 5762:                             }
 5763:                             emit_stream(&hdl, &rid, StreamEvent::TextDelta {
 5764:                                 session_id: sid.clone(),
 5765:                                 text: delta.clone(),
 5766:                                 order: Some(mark.seq),
 5767:                                 part_id: Some(mark.id.clone()),
 5768:                                 render_seq: Some(mark.seq),
 5769:                             });
 5770:                             partial_for_text.append_text(&delta);
 5771:                             if let Some(ref parent) = ptc {
 5772:                                 emit_parent_stream(&hdl, parent.tool_call_delta(delta));
 5773:                             }
 5774:                         },
 5775:                         move |thinking| {
 5776:                             emitted_output_for_thinking.store(true, Ordering::Relaxed);
 5777:                             let mark = render_order_for_thinking
 5778:                                 .lock()
 5779:                                 .map(|mut tracker| {
 5780:                                     tracker.mark_thinking(&rid2, &thinking_block_id)
 5781:                                 })
 5782:                                 .unwrap_or(RenderPartMark {
 5783:                                     id: format!("{}:thinking:{}", rid2, thinking_block_id),
 5784:                                     seq: 1,
 5785:                                 });
 5786:                             if !first_thinking_delta_logged_for_cb.swap(true, Ordering::Relaxed) {
 5787:                                 eprintln!(
 5788:                                     "[Agent {}] first thinking delta: session={} run={} iteration={} attempt={}/{} elapsed_ms={} delta_len={}",
 5789:                                     agent_id_for_thinking,
 5790:                                     sid2,
 5791:                                     rid2,
 5792:                                     iteration,
 5793:                                     attempt_number,
 5794:                                     LLM_RETRIES + 1,
 5795:                                     llm_call_started_at.elapsed().as_millis(),
 5796:                                     thinking.len()
 5797:                                 );
 5798:                             }
 5799:                             emit_stream(&hdl2, &rid2, StreamEvent::ThinkingDelta {
 5800:                                 session_id: sid2.clone(),
 5801:                                 text: thinking.clone(),
 5802:                                 order: Some(mark.seq),
 5803:                                 part_id: Some(mark.id.clone()),
 5804:                                 render_seq: Some(mark.seq),
 5805:                             });
 5806:                             partial_for_thinking.append_thinking(&thinking);
 5807:                         },
 5808:                         move |tool_call_id, tool_name| {
 5809:                             let tool_name = tool_registry_for_tool_start
 5810:                                 .canonical_name(&tool_name)
 5811:                                 .map(str::to_string)
 5812:                                 .unwrap_or(tool_name);
 5813:                             emitted_output_for_tool.store(true, Ordering::Relaxed);
 5814:                             let mark = render_order_for_tool
 5815:                                 .lock()
 5816:                                 .map(|mut tracker| tracker.mark_tool(&rid3, &tool_call_id))
 5817:                                 .unwrap_or(RenderPartMark {
 5818:                                     id: tool_call_id.clone(),
 5819:                                     seq: 1,
 5820:                                 });
 5821:                             if !first_tool_call_logged_for_cb.swap(true, Ordering::Relaxed) {
 5822:                                 eprintln!(
 5823:                                     "[Agent {}] first tool call start: session={} run={} iteration={} attempt={}/{} elapsed_ms={} tool_call_id={} tool_name={}",
 5824:                                     agent_id_for_tool_start,
 5825:                                     sid3,
 5826:                                     rid3,
 5827:                                     iteration,
 5828:                                     attempt_number,
 5829:                                     LLM_RETRIES + 1,
 5830:                                     llm_call_started_at.elapsed().as_millis(),
 5831:                                     tool_call_id,
 5832:                                     tool_name
 5833:                                 );
 5834:                             }
 5835:                             emit_stream(&hdl3, &rid3, StreamEvent::ToolCallStart {
 5836:                                 session_id: sid3.clone(),
 5837:                                 tool_call_id: tool_call_id.clone(),
 5838:                                 tool_name: tool_name.clone(),
 5839:                                 arguments: String::new(),
 5840:                                 order: Some(mark.seq),
 5841:                                 part_id: Some(tool_call_id.clone()),
 5842:                                 render_seq: Some(mark.seq),
 5843:                             });
 5844:                             if let Some(ref parent) = ptc3 {
 5845:                                 emit_parent_stream(
 5846:                                     &hdl3,
 5847:                                     parent.subagent_tool_call_start(
 5848:                                         tool_call_id,
 5849:                                         tool_name,
 5850:                                         String::new(),
 5851:                                         Some(mark.seq),
 5852:                                         Some(mark.id),
 5853:                                         Some(mark.seq),
 5854:                                     ),
 5855:                                 );
````

### src-tauri/src/agent/instance/mod.rs:6080-6590

````rust
 6080:             let mut ordered_tool_calls = render_order_tracker
 6081:                 .lock()
 6082:                 .map(|mut tracker| {
 6083:                     tracker.assign_tool_orders_for_run(&run_id, &response.tool_calls)
 6084:                 })
 6085:                 .unwrap_or_else(|_| response.tool_calls.clone());
 6086:             self.normalize_tool_call_names(&mut ordered_tool_calls);
 6087:             let response_content_order = response_text_part.as_ref().map(|part| part.seq);
 6088:             let response_thinking_order = response_thinking_part.as_ref().map(|part| part.seq);
 6089:             let response_render_parts = assistant_render_parts_for_response(
 6090:                 &run_id,
 6091:                 response_text_part.clone(),
 6092:                 &response.text,
 6093:                 response_thinking_part.clone(),
 6094:                 &response.thinking_text,
 6095:                 (response.thinking_duration_secs > 0).then_some(response.thinking_duration_secs),
 6096:                 (!response.thinking_signature.is_empty()).then_some(response.thinking_signature.as_str()),
 6097:                 &ordered_tool_calls,
 6098:             );
 6099: 
 6100:             if response.input_tokens > 0 || response.output_tokens > 0
 6101:                 || response.cache_read_tokens > 0 || response.cache_write_tokens > 0
 6102:             {
 6103:                 let priced_rounds = if matches!(&self.backend, LlmBackend::OpenRouter { .. }) {
 6104:                     1
 6105:                 } else {
 6106:                     0
 6107:                 };
 6108:                 let context_tokens = response.input_tokens
 6109:                     + response.cache_read_tokens
 6110:                     + response.cache_write_tokens
 6111:                     + response.output_tokens;
 6112:                 let context_limit = if let LlmBackend::Custom { context_length, .. } = &self.backend {
 6113:                     *context_length
 6114:                 } else {
 6115:                     model_context_limit(&self.effective_model)
 6116:                 };
 6117:                 match store.record_token_usage(
 6118:                     &self.session_id,
 6119:                     response.input_tokens as u64,
 6120:                     response.output_tokens as u64,
 6121:                     response.cache_read_tokens as u64,
 6122:                     response.cache_write_tokens as u64,
 6123:                     response.cost_usd,
 6124:                     priced_rounds,
 6125:                     Some(context_tokens),
 6126:                     Some(context_limit),
 6127:                 ) {
 6128:                     Ok(totals) => {
 6129:                         eprintln!(
 6130:                             "[Agent {}] tokens: +{}in/+{}out/+{}cache_r/+{}cache_w, cost=${:.6}, total: {}in/{}out/{}cache_r/{}cache_w/${:.6}",
 6131:                             self.id,
 6132:                             response.input_tokens, response.output_tokens,
 6133:                             response.cache_read_tokens, response.cache_write_tokens,
 6134:                             response.cost_usd,
 6135:                             totals.total_input_tokens, totals.total_output_tokens,
 6136:                             totals.total_cache_read_tokens, totals.total_cache_write_tokens,
 6137:                             totals.total_cost_usd,
 6138:                         );
 6139:                         emit_stream(app_handle, &run_id, StreamEvent::UsageUpdate {
 6140:                             session_id: self.session_id.clone(),
 6141:                             input_tokens: response.input_tokens,
 6142:                             output_tokens: response.output_tokens,
 6143:                             cache_read_tokens: response.cache_read_tokens,
 6144:                             cache_write_tokens: response.cache_write_tokens,
 6145:                             total_input_tokens: totals.total_input_tokens,
 6146:                             total_output_tokens: totals.total_output_tokens,
 6147:                             total_cache_read_tokens: totals.total_cache_read_tokens,
 6148:                             total_cache_write_tokens: totals.total_cache_write_tokens,
 6149:                             total_cost_usd: totals.total_cost_usd,
 6150:                             priced_rounds: totals.priced_rounds,
 6151:                             context_tokens,
 6152:                             context_limit,
 6153:                         });
 6154:                     }
 6155:                     Err(e) => {
 6156:                         eprintln!("[Agent {}] failed to record token usage: {}", self.id, e);
 6157:                     }
 6158:                 }
 6159:             }
 6160: 
 6161:             // Emit ToolCallStart (with arguments) + ToolCallDone for server tool calls (e.g. web_search)
 6162:             // that have pre-computed output. These don't need local execution. Output is embedded
 6163:             // as text in the assistant message for API history, so no separate Tool message is needed.
 6164:             for tc in &ordered_tool_calls {
 6165:                 if let Some(ref output) = tc.server_tool_output {
 6166:                     eprintln!(
 6167:                         "[Agent {}] server tool '{}' (id={}) has pre-computed output ({} chars)",
 6168:                         self.id, tc.name, tc.id, output.len()
 6169:                     );
 6170:                     // Re-emit ToolCallStart with arguments so the frontend can display them.
 6171:                     emit_stream(app_handle, &run_id, StreamEvent::ToolCallStart {
 6172:                         session_id: self.session_id.clone(),
 6173:                         tool_call_id: tc.id.clone(),
 6174:                         tool_name: tc.name.clone(),
 6175:                         arguments: tc.arguments.clone(),
 6176:                         order: tc.order,
 6177:                         part_id: Some(tc.id.clone()),
 6178:                         render_seq: tc.order,
 6179:                     });
 6180:                     emit_stream(app_handle, &run_id, StreamEvent::ToolCallDone {
 6181:                         session_id: self.session_id.clone(),
 6182:                         tool_call_id: tc.id.clone(),
 6183:                         tool_name: tc.name.clone(),
 6184:                         output: output.clone(),
 6185:                         outcome: crate::commands::ToolCallOutcome::Done,
 6186:                     });
 6187:                     if let Some(ref parent) = self.parent_tool_call {
 6188:                         emit_parent_stream(
 6189:                             app_handle,
 6190:                             parent.subagent_tool_call_start(
 6191:                                 tc.id.clone(),
 6192:                                 tc.name.clone(),
 6193:                                 tc.arguments.clone(),
 6194:                                 tc.order,
 6195:                                 Some(tc.id.clone()),
 6196:                                 tc.order,
 6197:                             ),
 6198:                         );
 6199:                         emit_parent_stream(
 6200:                             app_handle,
 6201:                             parent.subagent_tool_call_done(
 6202:                                 tc.id.clone(),
 6203:                                 tc.name.clone(),
 6204:                                 output.clone(),
 6205:                                 crate::commands::ToolCallOutcome::Done,
 6206:                             ),
 6207:                         );
 6208:                     }
 6209:                 }
 6210:             }
 6211: 
 6212:             let has_executable_tool_calls = ordered_tool_calls.iter()
 6213:                 .any(|tc| !tc.is_server_tool());
 6214: 
 6215:             if !ordered_tool_calls.is_empty() {
 6216:                 eprintln!(
 6217:                     "[Agent {}] got {} tool calls ({} executable, {} server)",
 6218:                     self.id,
 6219:                     ordered_tool_calls.len(),
 6220:                     ordered_tool_calls.iter().filter(|tc| !tc.is_server_tool()).count(),
 6221:                     ordered_tool_calls.iter().filter(|tc| tc.is_server_tool()).count(),
 6222:                 );
 6223: 
 6224:                 let thinking_opt = if response.thinking_text.is_empty() { None } else { Some(response.thinking_text.as_str()) };
 6225:                 let thinking_dur = if response.thinking_duration_secs > 0 { Some(response.thinking_duration_secs) } else { None };
 6226:                 let thinking_sig = if response.thinking_signature.is_empty() { None } else { Some(response.thinking_signature.as_str()) };
 6227:                 let assistant_msg_id = store.add_assistant_with_tool_calls_and_render_parts(
 6228:                     &self.session_id,
 6229:                     &response.text,
 6230:                     &ordered_tool_calls,
 6231:                     thinking_opt,
 6232:                     thinking_dur,
 6233:                     thinking_sig,
 6234:                     response.response_id.as_deref(),
 6235:                     response.continuation_request.as_ref(),
 6236:                     response_content_order,
 6237:                     response_thinking_order,
 6238:                     &response_render_parts,
 6239:                 )?;
 6240:                 self.partial_assistant.mark_persisted(
 6241:                     assistant_msg_id.clone(),
 6242:                     response.text.clone(),
 6243:                     thinking_opt.map(str::to_string),
 6244:                     thinking_dur,
 6245:                 );
 6246: 
 6247:                 let mut prepared: Vec<(ToolCallInfo, serde_json::Value)> = Vec::new();
 6248:                 for tc in &ordered_tool_calls {
 6249:                     // Skip server tools that already have pre-computed output.
 6250:                     if tc.is_server_tool() {
 6251:                         continue;
 6252:                     }
 6253: 
 6254:                     eprintln!(
 6255:                         "[Agent {}] executing tool '{}' (id={})",
 6256:                         self.id, tc.name, tc.id
 6257:                     );
 6258: 
 6259:                     emit_stream(app_handle, &run_id, StreamEvent::ToolCallStart {
 6260:                         session_id: self.session_id.clone(),
 6261:                         tool_call_id: tc.id.clone(),
 6262:                         tool_name: tc.name.clone(),
 6263:                         arguments: tc.arguments.clone(),
 6264:                         order: tc.order,
 6265:                         part_id: Some(tc.id.clone()),
 6266:                         render_seq: tc.order,
 6267:                     });
 6268:                     if let Some(ref parent) = self.parent_tool_call {
 6269:                         emit_parent_stream(
 6270:                             app_handle,
 6271:                             parent.subagent_tool_call_start(
 6272:                                 tc.id.clone(),
 6273:                                 tc.name.clone(),
 6274:                                 tc.arguments.clone(),
 6275:                                 tc.order,
 6276:                                 Some(tc.id.clone()),
 6277:                                 tc.order,
 6278:                             ),
 6279:                         );
 6280:                     }
 6281: 
 6282:                     let mut args: serde_json::Value = match serde_json::from_str(&tc.arguments) {
 6283:                         Ok(v) => v,
 6284:                         Err(parse_err) if tc.arguments.trim().is_empty() => {
 6285:                             eprintln!(
 6286:                                 "[Agent {}] tool '{}' emitted empty arguments payload; defaulting to {{}}",
 6287:                                 self.id, tc.name
 6288:                             );
 6289:                             let _ = parse_err;
 6290:                             serde_json::json!({})
 6291:                         }
 6292:                         Err(parse_err) => {
 6293:                             eprintln!(
 6294:                                 "[Agent {}] tool '{}' arguments JSON parse failed: {} | raw({} chars): {}",
 6295:                                 self.id, tc.name, parse_err,
 6296:                                 tc.arguments.len(),
 6297:                                 &tc.arguments[..tc.arguments.len().min(200)]
 6298:                             );
 6299:                             let mut fallback = serde_json::json!({});
 6300:                             fallback["__parse_error"] = serde_json::Value::String(
 6301:                                 format!(
 6302:                                     "Tool arguments JSON was truncated or malformed during streaming (received {} chars). Parse error: {}. Please retry this tool call with the same arguments.",
 6303:                                     tc.arguments.len(), parse_err
 6304:                                 )
 6305:                             );
 6306:                             fallback
 6307:                         }
 6308:                     };
 6309:                     normalize_tool_args(&mut args);
 6310:                     self.inject_working_dir(&tc.name, &mut args);
 6311:                     prepared.push((tc.clone(), args));
 6312:                 }
 6313: 
 6314:                 let needs_undo = prepared
 6315:                     .iter()
 6316:                     .any(|(tc, _)| Self::needs_undo_tracking(&tc.name));
 6317:                 let has_unity_execute = prepared
 6318:                     .iter()
 6319:                     .any(|(tc, _)| tc.name == "unity_execute" || tc.name == "unity_run_states");
 6320: 
 6321:                 let pre_checkpoint = if needs_undo {
 6322:                     if let Some(ref undo_mgr) = self.undo_manager {
 6323:                         match undo_mgr.before_round(&self.working_dir, "agent round").await {
 6324:                             Ok(cp) => cp,
 6325:                             Err(e) => {
 6326:                                 eprintln!("[Agent {}] undo checkpoint failed: {}", self.id, e);
 6327:                                 let lower = e.to_ascii_lowercase();
 6328:                                 let message = if lower.contains("unable to index file 'nul'")
 6329:                                     || lower.contains("short read while indexing nul")
 6330:                                 {
 6331:                                     "Undo is unavailable for this round because Git could not snapshot the workspace. Remove or rename reserved Windows file names such as NUL in the repository."
 6332:                                 } else {
 6333:                                     "Undo may be unavailable for this round because the workspace snapshot failed."
 6334:                                 };
 6335:                                 crate::error::AppError::emit_background(
 6336:                                     app_handle,
 6337:                                     &crate::error::AppError::new(
 6338:                                         "undo.checkpoint_failed",
 6339:                                         message,
 6340:                                     )
 6341:                                     .detail(e)
 6342:                                     .operation("undo")
 6343:                                     .severity(crate::error::ErrorSeverity::Warning),
 6344:                                 );
 6345:                                 None
 6346:                             }
 6347:                         }
 6348:                     } else { None }
 6349:                 } else { None };
 6350: 
 6351:                 let has_unity_asset_writes = crate::unity_bridge::is_unity_project(&self.working_dir)
 6352:                     && prepared
 6353:                         .iter()
 6354:                         .any(|(tc, args)| self.is_unity_asset_write_call(tc, args));
 6355:                 if has_unity_asset_writes {
 6356:                     match crate::unity_bridge::begin_edit_session(&self.working_dir, &self.session_id).await {
 6357:                         Ok(msg) => eprintln!(
 6358:                             "[Agent {}] Unity edit session active for {}: {}",
 6359:                             self.id, self.session_id, msg
 6360:                         ),
 6361:                         Err(e) => eprintln!(
 6362:                             "[Agent {}] failed to begin Unity edit session for {}: {}",
 6363:                             self.id, self.session_id, e
 6364:                         ),
 6365:                     }
 6366:                 }
 6367: 
 6368:                 let has_unity_recompile = prepared.iter().any(|(tc, _)| tc.name == "unity_recompile");
 6369:                 let results = if has_unity_recompile {
 6370:                     eprintln!(
 6371:                         "[Agent {}] executing tool round sequentially because unity_recompile is a barrier",
 6372:                         self.id
 6373:                     );
 6374:                     let mut results = Vec::with_capacity(prepared.len());
 6375:                     let mut queued_asset_paths: Vec<String> = Vec::new();
 6376:                     for (tc, args) in &prepared {
 6377:                         if tc.name == "unity_recompile" && !queued_asset_paths.is_empty() {
 6378:                             match crate::unity_bridge::import_assets(&self.working_dir, &queued_asset_paths).await {
 6379:                                 Ok(msg) => eprintln!(
 6380:                                     "[Agent {}] queued changed Unity assets before recompile: {}",
 6381:                                     self.id, msg
 6382:                                 ),
 6383:                                 Err(e) => eprintln!(
 6384:                                     "[Agent {}] failed to queue changed Unity assets before recompile: {}",
 6385:                                     self.id, e
 6386:                                 ),
 6387:                             }
 6388:                             queued_asset_paths.clear();
 6389:                         }
 6390: 
 6391:                         let result = self.execute_single_tool(app_handle, store, tc, args, &run_id, &mode).await;
 6392:                         if let Some(asset_path) = self.unity_asset_relative_path(tc, args, &result) {
 6393:                             queued_asset_paths.push(asset_path);
 6394:                         }
 6395:                         results.push(result);
 6396:                     }
 6397: 
 6398:                     if !queued_asset_paths.is_empty() {
 6399:                         crate::unity_bridge::import_assets_fire_and_forget(
 6400:                             &self.working_dir,
 6401:                             queued_asset_paths,
 6402:                         );
 6403:                     }
 6404:                     results
 6405:                 } else {
 6406:                     let mode_ref = mode.as_str();
 6407:                     let futures: Vec<_> = prepared.iter().map(|(tc, args)| {
 6408:                         self.execute_single_tool(app_handle, store, tc, args, &run_id, mode_ref)
 6409:                     }).collect();
 6410:                     futures::future::join_all(futures).await
 6411:                 };
 6412: 
 6413:                 if !has_unity_recompile {
 6414:                     let queued_asset_paths: Vec<String> = prepared
 6415:                         .iter()
 6416:                         .zip(results.iter())
 6417:                         .filter_map(|((tc, args), result)| {
 6418:                             self.unity_asset_relative_path(tc, args, result)
 6419:                         })
 6420:                         .collect();
 6421: 
 6422:                     if !queued_asset_paths.is_empty() {
 6423:                         crate::unity_bridge::import_assets_fire_and_forget(
 6424:                             &self.working_dir,
 6425:                             queued_asset_paths,
 6426:                         );
 6427:                     }
 6428:                 }
 6429: 
 6430:                 for ((tc, _), result) in prepared.iter().zip(results.iter()) {
 6431:                     let stored_output = match store.rewrite_tool_result_for_storage(
 6432:                         &self.session_id,
 6433:                         &tc.id,
 6434:                         &tc.name,
 6435:                         &result.output,
 6436:                     ) {
 6437:                         Ok(output) => output,
 6438:                         Err(e) => {
 6439:                             eprintln!(
 6440:                                 "[Agent {}] failed to persist tool_result for '{}' (id={}): {}",
 6441:                                 self.id, tc.name, tc.id, e
 6442:                             );
 6443:                             result.output.clone()
 6444:                         }
 6445:                     };
 6446:                     eprintln!(
 6447:                         "[Agent {}] tool '{}' result: outcome={:?}, is_error={}, output_len={} (stored={})",
 6448:                         self.id,
 6449:                         tc.name,
 6450:                         result.outcome,
 6451:                         result.is_error,
 6452:                         result.output.len(),
 6453:                         stored_output.len()
 6454:                     );
 6455: 
 6456:                     emit_stream(app_handle, &run_id, StreamEvent::ToolCallDone {
 6457:                         session_id: self.session_id.clone(),
 6458:                         tool_call_id: tc.id.clone(),
 6459:                         tool_name: tc.name.clone(),
 6460:                         output: stored_output.clone(),
 6461:                         outcome: result.outcome.as_stream_outcome(),
 6462:                     });
 6463:                     if let Some(ref parent) = self.parent_tool_call {
 6464:                         let truncated_output = if stored_output.chars().count() > 500 {
 6465:                             let s: String = stored_output.chars().take(500).collect();
 6466:                             format!("{}…({} chars)", s, result.output.chars().count())
 6467:                         } else {
 6468:                             stored_output.clone()
 6469:                         };
 6470:                         emit_parent_stream(
 6471:                             app_handle,
 6472:                             parent.subagent_tool_call_done(
 6473:                                 tc.id.clone(),
 6474:                                 tc.name.clone(),
 6475:                                 truncated_output,
 6476:                                 result.outcome.as_stream_outcome(),
 6477:                             ),
 6478:                         );
 6479:                     }
 6480: 
 6481:                     if let Err(e) = store.add_tool_result(
 6482:                         &self.session_id,
 6483:                         &tc.id,
 6484:                         &stored_output,
 6485:                     ) {
 6486:                         eprintln!(
 6487:                             "[Agent {}] failed to save tool_result for '{}' (id={}): {}",
 6488:                             self.id, tc.name, tc.id, e
 6489:                         );
 6490:                     }
 6491:                 }
 6492: 
 6493:                 let results_by_id: BTreeMap<&str, &ExecutedToolResult> = prepared
 6494:                     .iter()
 6495:                     .zip(results.iter())
 6496:                     .map(|((tool_call, _), result)| (tool_call.id.as_str(), result))
 6497:                     .collect();
 6498:                 let finalized_tool_calls: Vec<ToolCallInfo> = ordered_tool_calls
 6499:                     .iter()
 6500:                     .map(|tool_call| {
 6501:                         finalize_tool_call_record(
 6502:                             tool_call,
 6503:                             results_by_id.get(tool_call.id.as_str()).copied(),
 6504:                         )
 6505:                     })
 6506:                     .collect();
 6507: 
 6508:                 let finalized_render_parts = assistant_render_parts_for_response(
 6509:                     &run_id,
 6510:                     response_text_part.clone(),
 6511:                     &response.text,
 6512:                     response_thinking_part.clone(),
 6513:                     &response.thinking_text,
 6514:                     (response.thinking_duration_secs > 0)
 6515:                         .then_some(response.thinking_duration_secs),
 6516:                     (!response.thinking_signature.is_empty())
 6517:                         .then_some(response.thinking_signature.as_str()),
 6518:                     &finalized_tool_calls,
 6519:                 );
 6520: 
 6521:                 if let Err(e) = store.update_message_tool_calls_and_render_parts(
 6522:                     &assistant_msg_id,
 6523:                     &finalized_tool_calls,
 6524:                     &finalized_render_parts,
 6525:                 ) {
 6526:                     eprintln!(
 6527:                         "[Agent {}] failed to update tool_calls/render_parts for assistant message {}: {}",
 6528:                         self.id, assistant_msg_id, e
 6529:                     );
 6530:                 }
 6531: 
 6532:                 if let Some(checkpoint) = pre_checkpoint {
 6533:                     if let Some(ref undo_mgr) = self.undo_manager {
 6534:                         let recorded = undo_mgr
 6535:                             .after_round(
 6536:                                 &self.session_id,
 6537:                                 &assistant_msg_id,
 6538:                                 Some(run_id.as_str()),
 6539:                                 checkpoint,
 6540:                                 has_unity_execute,
 6541:                                 &self.working_dir,
 6542:                             )
 6543:                             .await;
 6544:                         match recorded {
 6545:                             Ok(true) => {
 6546:                                 eprintln!(
 6547:                                     "[Agent {}] emitting UndoAvailable for session {} run {} message {}",
 6548:                                     self.id, self.session_id, run_id, assistant_msg_id
 6549:                                 );
 6550:                                 emit_stream(app_handle, &run_id, StreamEvent::UndoAvailable {
 6551:                                     session_id: self.session_id.clone(),
 6552:                                     assistant_message_id: assistant_msg_id.clone(),
 6553:                                 });
 6554:                             }
 6555:                             Ok(false) => {}
 6556:                             Err(e) => {
 6557:                                 eprintln!(
 6558:                                     "[Agent {}] failed to record undo state for session {} message {}: {}",
 6559:                                     self.id, self.session_id, assistant_msg_id, e
 6560:                                 );
 6561:                                 crate::error::AppError::emit_background(
 6562:                                     app_handle,
 6563:                                     &crate::error::AppError::new(
 6564:                                         "undo.record_failed",
 6565:                                         "Undo may be unavailable for this round because file-change capture failed.",
 6566:                                     )
 6567:                                     .detail(e)
 6568:                                     .operation("undo")
 6569:                                     .severity(crate::error::ErrorSeverity::Warning),
 6570:                                 );
 6571:                             }
 6572:                         }
 6573:                     }
 6574:                 }
 6575: 
 6576:                 emit_stream(app_handle, &run_id, StreamEvent::ToolCallRoundDone {
 6577:                     session_id: self.session_id.clone(),
 6578:                     message_id: assistant_msg_id.clone(),
 6579:                     full_text: response.text.clone(),
 6580:                     tool_calls: finalized_tool_calls,
 6581:                     content_order: response_content_order,
 6582:                     thinking_order: response_thinking_order,
 6583:                     render_parts: Some(finalized_render_parts),
 6584:                 });
 6585:                 self.partial_assistant.reset();
 6586: 
 6587:                 if self.is_cancel_requested() {
 6588:                     self.clear_pending_knowledge_proposal(app_handle).await;
 6589:                     self.emit_cancelled(app_handle, store, &run_id);
 6590:                     return Ok(String::new());
````

### src-tauri/src/agent/instance/mod.rs:6630-6760

````rust
 6630:             } else {
 6631:                 None
 6632:             };
 6633:             let thinking_sig = if final_thinking_signature.is_empty() {
 6634:                 None
 6635:             } else {
 6636:                 Some(final_thinking_signature.as_str())
 6637:             };
 6638:             let final_render_parts = assistant_render_parts_for_response(
 6639:                 &run_id,
 6640:                 final_content_order.map(|seq| RenderPartMark {
 6641:                     id: format!("{}:text:final", run_id),
 6642:                     seq,
 6643:                 }),
 6644:                 &final_text,
 6645:                 final_thinking_order.map(|seq| RenderPartMark {
 6646:                     id: format!("{}:thinking:final", run_id),
 6647:                     seq,
 6648:                 }),
 6649:                 thinking_opt.unwrap_or_default(),
 6650:                 thinking_dur,
 6651:                 thinking_sig,
 6652:                 &[],
 6653:             );
 6654:             let msg_id = store.add_message_with_thinking_and_render_parts(
 6655:                 &self.session_id,
 6656:                 MessageRole::Assistant,
 6657:                 &final_text,
 6658:                 thinking_opt,
 6659:                 thinking_dur,
 6660:                 thinking_sig,
 6661:                 final_response_id.as_deref(),
 6662:                 final_continuation_request.as_ref(),
 6663:                 final_content_order,
 6664:                 final_thinking_order,
 6665:                 &final_render_parts,
 6666:             )?;
 6667:             self.partial_assistant.mark_persisted(
 6668:                 msg_id.clone(),
 6669:                 final_text.clone(),
 6670:                 thinking_opt.map(str::to_string),
 6671:                 thinking_dur,
 6672:             );
 6673: 
 6674:             if let Err(error) = store.set_latest_completed_run_id(&self.session_id, Some(&run_id)) {
 6675:                 eprintln!(
 6676:                     "[Agent {}] failed to persist latest completed run id for session {} run {}: {}",
 6677:                     self.id, self.session_id, run_id, error
 6678:                 );
 6679:                 crate::error::AppError::emit_background(
 6680:                     app_handle,
 6681:                     &crate::error::AppError::new(
 6682:                         "session.latest_run_persist_failed",
 6683:                         "Latest run boundary may be unavailable for this session.",
 6684:                     )
 6685:                     .detail(error)
 6686:                     .operation("session")
 6687:                     .severity(crate::error::ErrorSeverity::Warning),
 6688:                 );
 6689:             }
 6690: 
 6691:             eprintln!(
 6692:                 "[Agent {}] emitting Done for session {} run {} message {} text_len={}",
 6693:                 self.id,
 6694:                 self.session_id,
 6695:                 run_id,
 6696:                 msg_id,
 6697:                 final_text.len()
 6698:             );
 6699:             emit_stream(
 6700:                 app_handle,
 6701:                 &run_id,
 6702:                 StreamEvent::Done {
 6703:                     session_id: self.session_id.clone(),
 6704:                     message_id: msg_id,
 6705:                     full_text: final_text.clone(),
 6706:                     content_order: final_content_order,
 6707:                     thinking_order: final_thinking_order,
 6708:                     render_parts: Some(final_render_parts),
 6709:                 },
 6710:             );
 6711:             self.partial_assistant.reset();
 6712:         } else {
 6713:             // Server-tool-only rounds already persisted their assistant message via
 6714:             // ToolCallRoundDone. The explicit Done event still needs to fire with the
 6715:             // same message id so the frontend can clear its in-flight run state while
 6716:             // still seeing the terminal response text.
 6717:             let terminal_message_id = terminal_done_message_id.clone().unwrap_or_default();
 6718: 
 6719:             if let Err(error) = store.set_latest_completed_run_id(&self.session_id, Some(&run_id)) {
 6720:                 eprintln!(
 6721:                     "[Agent {}] failed to persist latest completed run id for session {} run {}: {}",
 6722:                     self.id, self.session_id, run_id, error
 6723:                 );
 6724:                 crate::error::AppError::emit_background(
 6725:                     app_handle,
 6726:                     &crate::error::AppError::new(
 6727:                         "session.latest_run_persist_failed",
 6728:                         "Latest run boundary may be unavailable for this session.",
 6729:                     )
 6730:                     .detail(error)
 6731:                     .operation("session")
 6732:                     .severity(crate::error::ErrorSeverity::Warning),
 6733:                 );
 6734:             }
 6735: 
 6736:             eprintln!(
 6737:                 "[Agent {}] emitting Done for session {} run {} message {} (server-tool-only round) text_len={}",
 6738:                 self.id,
 6739:                 self.session_id,
 6740:                 run_id,
 6741:                 terminal_message_id,
 6742:                 final_text.len()
 6743:             );
 6744:             emit_stream(
 6745:                 app_handle,
 6746:                 &run_id,
 6747:                 StreamEvent::Done {
 6748:                     session_id: self.session_id.clone(),
 6749:                     message_id: terminal_message_id,
 6750:                     full_text: final_text.clone(),
 6751:                     content_order: final_content_order,
 6752:                     thinking_order: final_thinking_order,
 6753:                     render_parts: None,
 6754:                 },
 6755:             );
 6756:             self.partial_assistant.reset();
 6757:         }
 6758: 
 6759:         if let Err(error) = self
 6760:             .flush_pending_knowledge_proposal(app_handle, store, &run_id)
````

### src-tauri/src/commands/session.rs:1641-1702

````rust
 1641: pub async fn save_raw_context(
 1642:     session_id: String,
 1643:     file_path: String,
 1644:     include_system_prompt: bool,
 1645:     raw_store: State<'_, RawContextStore>,
 1646:     store: State<'_, Arc<SessionStore>>,
 1647:     workspace: State<'_, Arc<Workspace>>,
 1648:     registry: State<'_, Arc<AgentDefRegistry>>,
 1649: ) -> Result<String, AppError> {
 1650:     let working_dir = workspace.path.read().await.clone();
 1651:     let project_config = load_export_project_config(&working_dir);
 1652:     let usage = store.get_token_usage(&session_id).ok();
 1653:     let raw_markdown = {
 1654:         let raw = raw_store.lock().await;
 1655:         raw.get(&session_id)
 1656:             .filter(|rounds| !rounds.is_empty())
 1657:             .map(|rounds| {
 1658:                 format_rounds_as_markdown(
 1659:                     &session_id,
 1660:                     rounds,
 1661:                     usage.as_ref(),
 1662:                     project_config.as_ref(),
 1663:                     include_system_prompt,
 1664:                 )
 1665:             })
 1666:     };
 1667: 
 1668:     let (markdown, export_mode) = if let Some(markdown) = raw_markdown {
 1669:         (markdown, "raw-rounds")
 1670:     } else {
 1671:         let detail = store.load_session(&session_id)?;
 1672:         let todos = store
 1673:             .get_todos(&session_id)
 1674:             .map(|snapshot| snapshot.items)
 1675:             .unwrap_or_default();
 1676:         let system_prompt = if include_system_prompt {
 1677:             resolve_export_system_prompt(registry.inner(), detail.agent_id.as_deref())
 1678:         } else {
 1679:             None
 1680:         };
 1681:         (
 1682:             format_session_detail_as_markdown(
 1683:                 &detail,
 1684:                 &todos,
 1685:                 usage.as_ref(),
 1686:                 project_config.as_ref(),
 1687:                 include_system_prompt,
 1688:                 system_prompt.as_deref(),
 1689:             ),
 1690:             "session-store-fallback",
 1691:         )
 1692:     };
 1693: 
 1694:     std::fs::write(&file_path, markdown.as_bytes())
 1695:         .map_err(|e| format!("Failed to write file: {}", e))?;
 1696: 
 1697:     eprintln!(
 1698:         "[Locus] saved context export ({}, system_prompt={}) for session {} to {}",
 1699:         export_mode, include_system_prompt, session_id, file_path
 1700:     );
 1701:     Ok(file_path)
 1702: }
````

### src-tauri/src/commands/session.rs:1752-1915

````rust
 1752: const EMPTY_EXPORT_FIELD: &str = "empty";
 1753: 
 1754: fn append_project_config_markdown(out: &mut String, project_config: Option<&ExportProjectConfig>) {
 1755:     out.push_str("## Current Project Configuration\n\n");
 1756:     if let Some(config) = project_config {
 1757:         out.push_str(&format!("- **Workspace:** `{}`\n", config.working_dir));
 1758:         out.push_str(&format!(
 1759:             "- **Knowledge:** {}\n",
 1760:             format_enabled_state(config.knowledge_enabled)
 1761:         ));
 1762:         out.push_str(&format!(
 1763:             "- **Full-text Search:** {}\n",
 1764:             format_enabled_state(config.full_text_search_enabled)
 1765:         ));
 1766:         out.push_str(&format!(
 1767:             "- **Semantic Search:** {}\n",
 1768:             format_enabled_state(config.semantic_search_enabled)
 1769:         ));
 1770:     } else {
 1771:         out.push_str("- Project configuration unavailable: no workspace is currently selected.\n");
 1772:     }
 1773:     out.push_str("\n---\n\n");
 1774: }
 1775: 
 1776: fn extract_enabled_tools(rounds: &[crate::agent::instance::RawRound]) -> Vec<ExportEnabledTool> {
 1777:     let Some(tool_values) = rounds.iter().rev().find_map(|round| {
 1778:         round
 1779:             .request
 1780:             .get("tools")
 1781:             .and_then(|value| value.as_array())
 1782:     }) else {
 1783:         return Vec::new();
 1784:     };
 1785: 
 1786:     tool_values
 1787:         .iter()
 1788:         .filter_map(parse_export_enabled_tool)
 1789:         .collect()
 1790: }
 1791: 
 1792: fn parse_export_enabled_tool(value: &serde_json::Value) -> Option<ExportEnabledTool> {
 1793:     let function = value.get("function").unwrap_or(value);
 1794:     let name = function
 1795:         .get("name")
 1796:         .and_then(|field| field.as_str())
 1797:         .or_else(|| value.get("name").and_then(|field| field.as_str()))?
 1798:         .trim();
 1799:     if name.is_empty() {
 1800:         return None;
 1801:     }
 1802: 
 1803:     let description = function
 1804:         .get("description")
 1805:         .and_then(|field| field.as_str())
 1806:         .or_else(|| value.get("description").and_then(|field| field.as_str()))
 1807:         .unwrap_or("")
 1808:         .trim()
 1809:         .to_string();
 1810: 
 1811:     Some(ExportEnabledTool {
 1812:         name: name.to_string(),
 1813:         description,
 1814:     })
 1815: }
 1816: 
 1817: fn append_enabled_tools_markdown(out: &mut String, tools: &[ExportEnabledTool]) {
 1818:     out.push_str("## Enabled Tools\n\n");
 1819:     if tools.is_empty() {
 1820:         out.push_str("- No tools were enabled in the latest captured request.\n");
 1821:         out.push_str("\n---\n\n");
 1822:         return;
 1823:     }
 1824: 
 1825:     out.push_str(&format!("- **Count:** {}\n\n", tools.len()));
 1826:     for tool in tools {
 1827:         out.push_str(&format!("### `{}`\n\n", tool.name));
 1828:         if tool.description.is_empty() {
 1829:             out.push_str("*(No description provided)*\n\n");
 1830:         } else {
 1831:             out.push_str(&tool.description);
 1832:             out.push_str("\n\n");
 1833:         }
 1834:     }
 1835:     out.push_str("---\n\n");
 1836: }
 1837: 
 1838: fn format_export_timestamp(ts: i64) -> String {
 1839:     use chrono::{Local, TimeZone};
 1840: 
 1841:     Local
 1842:         .timestamp_opt(ts, 0)
 1843:         .single()
 1844:         .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
 1845:         .unwrap_or_else(|| ts.to_string())
 1846: }
 1847: 
 1848: fn export_optional_text(value: Option<&str>) -> serde_json::Value {
 1849:     let trimmed = value.unwrap_or("").trim();
 1850:     if trimmed.is_empty() {
 1851:         json!(EMPTY_EXPORT_FIELD)
 1852:     } else {
 1853:         json!(trimmed)
 1854:     }
 1855: }
 1856: 
 1857: fn export_optional_u32(value: Option<u32>) -> serde_json::Value {
 1858:     match value {
 1859:         Some(value) => json!(value),
 1860:         None => json!(EMPTY_EXPORT_FIELD),
 1861:     }
 1862: }
 1863: 
 1864: fn export_context_usage_value(value: u32, limit: u32) -> serde_json::Value {
 1865:     if limit > 0 {
 1866:         json!(value)
 1867:     } else {
 1868:         json!(EMPTY_EXPORT_FIELD)
 1869:     }
 1870: }
 1871: 
 1872: fn export_optional_tool_outcome(
 1873:     value: Option<crate::commands::ToolCallOutcome>,
 1874: ) -> serde_json::Value {
 1875:     match value {
 1876:         Some(value) => json!(value),
 1877:         None => json!(EMPTY_EXPORT_FIELD),
 1878:     }
 1879: }
 1880: 
 1881: fn export_optional_server_tool(
 1882:     value: Option<&crate::session::models::ServerToolKind>,
 1883: ) -> serde_json::Value {
 1884:     match value {
 1885:         Some(value) => json!(value),
 1886:         None => json!(EMPTY_EXPORT_FIELD),
 1887:     }
 1888: }
 1889: 
 1890: fn export_tool_call(tool_call: &crate::session::models::ToolCallInfo) -> serde_json::Value {
 1891:     json!({
 1892:         "id": tool_call.id,
 1893:         "name": tool_call.name,
 1894:         "arguments": tool_call.arguments,
 1895:         "order": export_optional_u32(tool_call.order),
 1896:         "serverTool": export_optional_server_tool(tool_call.server_tool.as_ref()),
 1897:         "serverToolOutput": export_optional_text(tool_call.server_tool_output.as_deref()),
 1898:         "outcome": export_optional_tool_outcome(tool_call.outcome),
 1899:         "recordedOutput": export_optional_text(tool_call.recorded_output.as_deref()),
 1900:         "nestedToolCalls": export_tool_calls(tool_call.nested_tool_calls.as_deref()),
 1901:     })
 1902: }
 1903: 
 1904: fn export_tool_calls(
 1905:     tool_calls: Option<&[crate::session::models::ToolCallInfo]>,
 1906: ) -> serde_json::Value {
 1907:     match tool_calls {
 1908:         Some(tool_calls) if !tool_calls.is_empty() => {
 1909:             json!(tool_calls.iter().map(export_tool_call).collect::<Vec<_>>())
 1910:         }
 1911:         _ => json!(EMPTY_EXPORT_FIELD),
 1912:     }
 1913: }
 1914: 
 1915: fn export_images(images: Option<&[ImageData]>) -> serde_json::Value {
````

### src-tauri/src/commands/session.rs:1990-2110

````rust
 1990:     detail: &SessionDetail,
 1991:     todos: &[TodoItem],
 1992:     usage: Option<&TokenUsage>,
 1993:     project_config: Option<&ExportProjectConfig>,
 1994:     include_system_prompt: bool,
 1995:     system_prompt: Option<&str>,
 1996: ) -> String {
 1997:     let mut out = String::with_capacity(16 * 1024);
 1998: 
 1999:     out.push_str("# Locus Conversation Log\n\n");
 2000:     out.push_str(&format!("- **Session:** `{}`\n", detail.id));
 2001:     out.push_str("- **Export Source:** `session-store-fallback`\n");
 2002:     out.push_str("- **Raw Rounds:** `empty`\n");
 2003:     out.push_str(&format!("- **Messages:** {}\n", detail.messages.len()));
 2004:     out.push_str(&format!(
 2005:         "- **Missing Field Marker:** `{}`\n\n",
 2006:         EMPTY_EXPORT_FIELD
 2007:     ));
 2008:     out.push_str(
 2009:         "## Export Note\n\nRaw request/response rounds were unavailable in memory for this session. \
 2010: This export was reconstructed from the persisted session store. Any field unavailable after \
 2011: migration is written as `empty`.\n\n",
 2012:     );
 2013:     if include_system_prompt {
 2014:         out.push_str(
 2015:             "System Prompt reflects the current agent definition for this session when available.\n\n",
 2016:         );
 2017:     }
 2018:     out.push_str("---\n\n");
 2019: 
 2020:     append_project_config_markdown(&mut out, project_config);
 2021:     if include_system_prompt {
 2022:         append_system_prompt_block(&mut out, system_prompt, 2);
 2023:         out.push_str("---\n\n");
 2024:     }
 2025: 
 2026:     let session_metadata = json!({
 2027:         "sessionId": detail.id,
 2028:         "title": export_optional_text(Some(&detail.title)),
 2029:         "agentId": export_optional_text(detail.agent_id.as_deref()),
 2030:         "sessionType": export_optional_text(Some(&detail.session_type)),
 2031:         "parentSessionId": export_optional_text(detail.parent_session_id.as_deref()),
 2032:         "latestCompletedRunId": export_optional_text(detail.latest_completed_run_id.as_deref()),
 2033:         "createdAtUnix": detail.created_at,
 2034:         "createdAtLocal": format_export_timestamp(detail.created_at),
 2035:         "updatedAtUnix": detail.updated_at,
 2036:         "updatedAtLocal": format_export_timestamp(detail.updated_at),
 2037:     });
 2038:     append_json_block(&mut out, "Session Metadata", &session_metadata, 2);
 2039: 
 2040:     let usage_json = match usage {
 2041:         Some(usage) => json!({
 2042:             "totalInputTokens": usage.total_input_tokens,
 2043:             "totalOutputTokens": usage.total_output_tokens,
 2044:             "totalCacheReadTokens": usage.total_cache_read_tokens,
 2045:             "totalCacheWriteTokens": usage.total_cache_write_tokens,
 2046:             "totalCostUsd": usage.total_cost_usd,
 2047:             "pricedRounds": usage.priced_rounds,
 2048:             "contextTokens": export_context_usage_value(usage.context_tokens, usage.context_limit),
 2049:             "contextLimit": export_context_usage_value(usage.context_limit, usage.context_limit),
 2050:         }),
 2051:         None => json!({
 2052:             "totalInputTokens": EMPTY_EXPORT_FIELD,
 2053:             "totalOutputTokens": EMPTY_EXPORT_FIELD,
 2054:             "totalCacheReadTokens": EMPTY_EXPORT_FIELD,
 2055:             "totalCacheWriteTokens": EMPTY_EXPORT_FIELD,
 2056:             "totalCostUsd": EMPTY_EXPORT_FIELD,
 2057:             "pricedRounds": EMPTY_EXPORT_FIELD,
 2058:             "contextTokens": EMPTY_EXPORT_FIELD,
 2059:             "contextLimit": EMPTY_EXPORT_FIELD,
 2060:         }),
 2061:     };
 2062:     append_json_block(&mut out, "Token Usage", &usage_json, 2);
 2063: 
 2064:     let todos_json = if todos.is_empty() {
 2065:         json!(EMPTY_EXPORT_FIELD)
 2066:     } else {
 2067:         json!(todos)
 2068:     };
 2069:     append_json_block(&mut out, "Todos", &todos_json, 2);
 2070: 
 2071:     out.push_str("## Messages\n\n");
 2072:     if detail.messages.is_empty() {
 2073:         out.push_str("`empty`\n\n");
 2074:         return out;
 2075:     }
 2076: 
 2077:     for (index, message) in detail.messages.iter().enumerate() {
 2078:         let metadata = json!({
 2079:             "messageIndex": index + 1,
 2080:             "id": message.id,
 2081:             "role": message.role,
 2082:             "createdAtUnix": message.created_at,
 2083:             "createdAtLocal": format_export_timestamp(message.created_at),
 2084:             "promptPrefix": export_optional_text(message.prompt_prefix.as_deref()),
 2085:             "promptSuffix": export_optional_text(message.prompt_suffix.as_deref()),
 2086:             "responseId": export_optional_text(message.response_id.as_deref()),
 2087:             "contentOrder": export_optional_u32(message.content_order),
 2088:             "thinkingOrder": export_optional_u32(message.thinking_order),
 2089:             "renderParts": message
 2090:                 .render_parts
 2091:                 .as_ref()
 2092:                 .map(|parts| json!(parts))
 2093:                 .unwrap_or_else(|| json!(EMPTY_EXPORT_FIELD)),
 2094:             "toolCalls": export_tool_calls(message.tool_calls.as_deref()),
 2095:             "toolCallId": export_optional_text(message.tool_call_id.as_deref()),
 2096:             "images": export_images(message.images.as_deref()),
 2097:             "assetRefs": export_asset_refs(message.asset_refs.as_deref()),
 2098:             "thinkingContent": export_optional_text(message.thinking_content.as_deref()),
 2099:             "thinkingDuration": export_optional_u32(message.thinking_duration),
 2100:             "thinkingSignature": export_optional_text(message.thinking_signature.as_deref()),
 2101:             "knowledgeProposal": message
 2102:                 .knowledge_proposal
 2103:                 .as_ref()
 2104:                 .map(|proposal| json!(proposal))
 2105:                 .unwrap_or_else(|| json!(EMPTY_EXPORT_FIELD)),
 2106:         });
 2107: 
 2108:         append_json_block(&mut out, &format!("Message {}", index + 1), &metadata, 3);
 2109:         append_text_block(&mut out, "Content", Some(&message.content), 4);
 2110:         out.push_str("---\n\n");
````

### src-tauri/src/commands/session.rs:2310-2360

````rust
 2310: 
 2311: fn format_request_history_items(out: &mut String, items: &[serde_json::Value]) {
 2312:     let mut index = 0usize;
 2313:     while index < items.len() {
 2314:         if is_request_function_call_item(&items[index]) {
 2315:             index = format_request_tool_call_batch(out, items, index);
 2316:             continue;
 2317:         }
 2318:         format_request_history_item(out, &items[index]);
 2319:         index += 1;
 2320:     }
 2321: }
 2322: 
 2323: fn is_request_function_call_item(item: &serde_json::Value) -> bool {
 2324:     item.get("type").and_then(|value| value.as_str()) == Some("function_call")
 2325: }
 2326: 
 2327: fn is_request_function_call_output_item(item: &serde_json::Value) -> bool {
 2328:     item.get("type").and_then(|value| value.as_str()) == Some("function_call_output")
 2329: }
 2330: 
 2331: fn format_request_tool_call_batch(
 2332:     out: &mut String,
 2333:     items: &[serde_json::Value],
 2334:     start_index: usize,
 2335: ) -> usize {
 2336:     let mut index = start_index;
 2337:     let mut tool_calls: Vec<ExportRequestToolCall<'_>> = Vec::new();
 2338:     while index < items.len() && is_request_function_call_item(&items[index]) {
 2339:         tool_calls.push(ExportRequestToolCall::from_item(&items[index]));
 2340:         index += 1;
 2341:     }
 2342: 
 2343:     let mut pending_outputs: Vec<ExportRequestToolOutput<'_>> = Vec::new();
 2344:     while index < items.len() && is_request_function_call_output_item(&items[index]) {
 2345:         pending_outputs.push(ExportRequestToolOutput::from_item(&items[index]));
 2346:         index += 1;
 2347:     }
 2348: 
 2349:     for tool_call in tool_calls {
 2350:         format_assistant_tool_call_message(out, tool_call.name, tool_call.arguments);
 2351: 
 2352:         let Some(call_id) = tool_call.call_id.filter(|value| !value.is_empty()) else {
 2353:             continue;
 2354:         };
 2355: 
 2356:         let mut remaining_outputs = Vec::with_capacity(pending_outputs.len());
 2357:         for tool_output in pending_outputs {
 2358:             if tool_output.call_id == Some(call_id) {
 2359:                 format_tool_output_message(out, tool_output.call_id, tool_output.output);
 2360:             } else {
````

### src/__tests__/chatSidebarLayout.test.ts:120-360

````ts
  120:     expect(transcript).toContain(".chat-transcript-message.is-session.assistant.transient.waiting-placeholder {");
  121:     expect(chatView).toContain("background: var(--msg-assistant-bg);");
  122:     expect(sidebar).toContain("background: var(--msg-assistant-bg);");
  123:     expect(todoPanel).toContain("background: var(--msg-assistant-bg);");
  124:     expect(changesPanel).toContain("background: var(--msg-assistant-bg);");
  125:     expect(toolCollection).toContain("var(--msg-assistant-bg)");
  126:   });
  127: 
  128:   it("animates tool batch collapse upward instead of dropping the list abruptly", () => {
  129:     const toolCollection = read("src/components/ToolCallCollection.vue");
  130: 
  131:     expect(toolCollection).toContain("const panelVisible = ref(false);");
  132:     expect(toolCollection).toContain("const panelLeaving = ref(false);");
  133:     expect(toolCollection).toContain("const summaryOpen = computed(() =>");
  134:     expect(toolCollection).toContain("height 320ms cubic-bezier(0.2, 0, 0, 1)");
  135:     expect(toolCollection).toContain("transformOrigin = \"top center\"");
  136:     expect(toolCollection).toContain("<Transition");
  137:     expect(toolCollection).toContain(":css=\"false\"");
  138:     expect(toolCollection).toContain("@leave=\"onPanelLeave\"");
  139:     expect(toolCollection).toContain("emit(\"collapseFinished\");");
  140:     expect(toolCollection).toContain("translateY(-4px) scaleY(0.97)");
  141:     expect(toolCollection).toContain("class=\"tool-call-collection-panel\"");
  142:     expect(toolCollection).toContain("'is-collapsing': batchState.canCollapse && panelLeaving");
  143:     expect(toolCollection).toContain(":class=\"{ open: expanded }\"");
  144:     expect(toolCollection).toContain(".tool-call-batch-summary.open.closing");
  145:   });
  146: 
  147:   it("keeps tool batches on a shared transient handoff path until the collapse leave finishes", () => {
  148:     const chatView = read("src/components/ChatView.vue");
  149:     const transcript = read("src/components/chat/ChatTranscript.vue");
  150:     const toolBlock = read("src/components/ToolCallBlock.vue");
  151:     const toolCollection = read("src/components/ToolCallCollection.vue");
  152: 
  153:     expect(transcript).toContain("interface ToolCallHandoffState {");
  154:     expect(transcript).toContain("const TOOL_HANDOFF_MIN_VISIBLE_MS = 160;");
  155:     expect(transcript).toContain("const hasVisibleStreamingText = computed(() => props.streamingText.trim().length > 0);");
  156:     expect(transcript).toContain("const shouldArmToolCallHandoffCollapse = computed(");
  157:     expect(transcript).toContain("const toolCallHandoff = ref<ToolCallHandoffState | null>(null);");
  158:     expect(transcript).toContain("renderKey: `tool-handoff-");
  159:     expect(transcript).toContain("collapseCandidateToolCalls: ToolCallDisplay[];");
  160:     expect(transcript).toContain("collapseFinished: boolean;");
  161:     expect(transcript).toContain("function shouldRetainCollapsedToolCallHandoff(handoff: ToolCallHandoffState)");
  162:     expect(transcript).toContain("return handoff.collapseFinished || (handoff.collapseArmed && handoff.willAutoCollapse);");
  163:     expect(transcript).toContain("collectToolCallDisplayMatchState(retainedToolCalls)");
  164:     expect(transcript).toContain("collapseCandidateToolCalls: cloneToolCallDisplays(transientCollapseCandidateToolCalls.value)");
  165:     expect(transcript).toContain("willAutoCollapse: summarizeToolCallBatch(toolCalls, displaySettings.compactToolCalls).canCollapse");
  166:     expect(transcript).toContain("setToolCallHandoffQuiet(true);");
  167:     expect(transcript).toContain("if (!transientToolCallsCanCollapse.value) {");
  168:     expect(transcript).toContain("if (shouldArmToolCallHandoffCollapse.value) {");
  169:     expect(transcript).toContain("watch(shouldArmToolCallHandoffCollapse, (shouldArm) => {");
  170:     expect(transcript).toContain("clearToolCallHandoff(\"stream-ended-after-collapse\")");
  171:     expect(transcript).toContain("const shouldPromoteHistoryToolCalls = computed(");
  172:     expect(transcript).toContain("props.activeToolCalls.length > 0");
  173:     expect(transcript).toContain("const promotableHistoryToolCalls = computed<PromotedHistoryToolCallsState>(() => {");
  174:     expect(transcript).toContain("const segments = historyRenderSegmentsForGroup(lastGroup);");
  175:     expect(transcript).toContain("if (!segment || segment.type !== \"toolCalls\") break;");
  176:     expect(transcript).toContain("const transientCollapseCandidateToolCalls = computed(() => {");
  177:     expect(transcript).toContain("if (promotableHistoryToolCalls.value.toolCalls.length === 0) {");
  178:     expect(transcript).toContain("const transientToolCallsCanCollapse = computed(() =>");
  179:     expect(transcript).toContain("const shouldHidePromotedHistoryToolCalls = computed(() =>");
  180:     expect(transcript).toContain("promotedHistoryToolCallsVisibilityChanged");
  181:     expect(transcript).toContain("const shouldRenderPromotedHistoryToolCallsInTransient = computed(() =>");
  182:     expect(transcript).toContain("transientPromotedToolCallsCoverage");
  183:     expect(transcript).toContain("promotedHistoryToolCallsRenderGap");
  184:     expect(transcript).toContain("missingPromotedToolCallIds");
  185:     expect(transcript).toContain("@collapse-finished=\"onTransientToolCallsCollapseFinished\"");
  186:     expect(transcript).toContain(":tool-calls=\"segment.toolCalls\"");
  187:     expect(transcript).toContain("function transientToolHandoffPart(toolCalls: ToolCallDisplay[])");
  188:     expect(transcript).toContain("let hasRenderedToolSegment = segments.some((segment) => segment.type === \"toolCalls\");");
  189:     expect(transcript).toContain("const promotedToolCalls = promotableHistoryToolCalls.value.toolCalls;");
  190:     expect(transcript).toContain("const firstToolSegmentIndex = segments.findIndex((segment) => segment.type === \"toolCalls\");");
  191:     expect(transcript).toContain("const firstContentSegmentIndex = segments.findIndex((segment) => segment.type === \"content\");");
  192:     expect(transcript).toContain("mergeToolCallDisplaysWithoutDuplicates(");
  193:     expect(transcript).toContain("firstToolSegment.key = transientToolSegmentKey(mergedToolCalls);");
  194:     expect(transcript).toContain("firstToolSegment.allowCollapse = true;");
  195:     expect(transcript).toContain("firstToolSegment.collapseEnabled = true;");
  196:     expect(transcript).toContain("segments.unshift({");
  197:     expect(transcript).toContain("allowCollapse: true,");
  198:     expect(transcript).toContain("collapseEnabled: true,");
  199:     expect(transcript).toContain("hasRenderedToolSegment = segments.some((segment) => segment.type === \"toolCalls\");");
  200:     expect(transcript).toContain("if (!hasRenderedToolSegment && transientToolCalls.value.length > 0) {");
  201:     expect(transcript).toContain("function transientToolSegmentKey(toolCalls: ToolCallDisplay[])");
  202:     expect(transcript).toContain("key: transientToolSegmentKey(pendingToolCalls),");
  203:     expect(transcript).toContain("key: transientToolSegmentKey(transientToolCalls.value),");
  204:     expect(transcript).toContain("animateCollapseOnMount: !!toolCallHandoff.value?.collapseArmed,");
  205:     expect(transcript).toContain(":animate-collapse-on-mount=\"segment.animateCollapseOnMount\"");
  206:     expect(transcript).toContain("if (!props.isStreaming && shouldReleaseToolCallHandoffToHistory(props.messages, previousMatchState))");
  207:     expect(transcript).toContain("if (!props.isStreaming && shouldReleaseToolCallHandoffToHistory(messages, toolCallHandoff.value.toolCallMatchState))");
  208:     expect(transcript).toContain(":allow-collapse=\"segment.allowCollapse\"");
  209:     expect(transcript).toContain(":collapse-enabled=\"segment.collapseEnabled\"");
  210:     expect(transcript).toContain(":collapse-enabled=\"segment.collapseEnabled\"");
  211:     expect(chatView).toContain("const toolHandoffViewportQuiet = ref(false);");
  212:     expect(chatView).toContain("function handleToolHandoffQuietChange(quiet: boolean) {");
  213:     expect(chatView).toContain("@tool-handoff-quiet-change=\"handleToolHandoffQuietChange\"");
  214:     expect(transcript).toContain(":allow-collapse=\"!shouldKeepToolSegmentExpanded(segment)\"");
  215:     expect(transcript).toContain(":collapse-enabled=\"!shouldKeepToolSegmentExpanded(segment)\"");
  216:     expect(toolBlock).toContain("collapseEnabled?: boolean;");
  217:     expect(toolBlock).toContain(":tool-calls=\"toolCall.nestedToolCalls\"");
  218:     expect(toolBlock).toContain(":collapse-enabled=\"collapseEnabled\"");
  219:     expect(toolBlock).toContain("@viewport-anchor-start=\"emitToolViewportAnchorStart\"");
  220:     expect(toolBlock).toContain("@tool-viewport-anchor-start=\"emitToolViewportAnchorStart\"");
  221:     expect(toolCollection).toContain("collapseEnabled?: boolean;");
  222:     expect(toolCollection).toContain("animateCollapseOnMount?: boolean;");
  223:     expect(toolCollection).toContain("const startsExpandedForCollapseAnimation =");
  224:     expect(toolCollection).toContain("onMounted(() => {");
  225:     expect(toolCollection).toContain("props.allowCollapse && props.collapseEnabled");
  226:   });
  227: 
  228:   it("keeps nested subagent tool rows compact", () => {
  229:     const toolBlock = read("src/components/ToolCallBlock.vue");
  230: 
  231:     expect(toolBlock).toContain(".nested-tool-calls :deep(.tool-call-header) {");
  232:     expect(toolBlock).toMatch(/\.nested-tool-calls :deep\(\.tool-call-header\)\s*\{[\s\S]*min-height:\s*18px/);
  233:     expect(toolBlock).toMatch(/\.nested-tool-calls :deep\(\.tool-call-collection-list\)\s*\{[\s\S]*gap:\s*2px/);
  234:     expect(toolBlock).toMatch(/\.nested-tool-calls :deep\(\.spinner-anim\)\s*\{[\s\S]*width:\s*8px/);
  235:   });
  236: 
  237:   it("auto-collapses completed subagent tool blocks", () => {
  238:     const toolBlock = read("src/components/ToolCallBlock.vue");
  239: 
  240:     expect(toolBlock).toContain("function shouldAutoExpandSubagentTool(toolCall: ToolCallDisplay) {");
  241:     expect(toolBlock).toContain("return isSubagentToolName(toolCall.name) && toolCall.status === \"running\";");
  242:     expect(toolBlock).toContain("const expanded = ref(shouldAutoExpandSubagentTool(props.toolCall));");
  243:     expect(toolBlock).toContain("if (previousStatus === \"running\" && nextStatus !== \"running\") {");
  244:     expect(toolBlock).toContain("setExpanded(false, true);");
  245:     expect(toolBlock).toContain("} else if (previousStatus !== \"running\" && nextStatus === \"running\") {");
  246:     expect(toolBlock).toContain("setExpanded(true, true);");
  247:   });
  248: 
  249:   it("filters history tool calls while the transient handoff batch owns the same ids", () => {
  250:     const chatView = read("src/components/ChatView.vue");
  251:     const transcript = read("src/components/chat/ChatTranscript.vue");
  252: 
  253:     expect(transcript).toContain("const hasLiveToolCalls = computed(() => props.activeToolCalls.length > 0);");
  254:     expect(transcript).toContain("const hasTransientToolCalls = computed(() => transientToolCalls.value.length > 0);");
  255:     expect(transcript).toContain("const hasToolCallHandoff = computed(() => hasTransientToolCalls.value && !hasLiveToolCalls.value);");
  256:     expect(transcript).toContain("const canonicalLiveRenderParts = computed(() => {");
  257:     expect(transcript).toContain("canonicalLiveRenderParts.value.some((part) => part.kind === \"text\" || part.kind === \"toolCall\")");
  258:     expect(transcript).toContain("const activeToolCallMatchState = computed<ToolCallMatchState>(() => {");
  259:     expect(transcript).toContain("return toolCallHandoff.value?.toolCallMatchState ?? {");
  260:     expect(transcript).toContain("const baseGroupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(activeToolCallMatchState.value));");
  261:     expect(transcript).toContain("const historyHiddenToolCallMatchState = computed<ToolCallMatchState>(() => {");
  262:     expect(transcript).toContain("return mergeToolCallMatchStates(");
  263:     expect(transcript).toContain("const groupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(historyHiddenToolCallMatchState.value));");
  264:     expect(transcript).toContain("toolCallTreeHasAnyIds(message.toolCalls, toolCallHandoff.value!.toolCallMatchState)");
  265:     expect(transcript).toContain("function shouldReleaseToolCallHandoffToHistory(");
  266:     expect(transcript).toContain("clearToolCallHandoff(\"handoff-followed-by-history-message\")");
  267:     expect(transcript).toContain("function buildTailHiddenToolCallMap(");
  268:     expect(transcript).toContain("filterToolCallsByConsumableMatchState(");
  269:     expect(transcript).toContain("cloneToolCallMatchState(hiddenToolCallMatchState)");
  270:     expect(chatView).toContain(":session-key=\"activeSessionId || NEW_CHAT_DRAFT_KEY\"");
  271:     expect(transcript).toContain("function shouldKeepToolItemExpanded(itemId: string) {");
  272:     expect(transcript).toContain("return nonCollapsibleToolItemIds.value.has(itemId);");
  273:     expect(transcript).toContain("if (toolCallHandoff.value?.collapseArmed) {");
  274:     expect(transcript).toContain("|| hasToolCallHandoff.value");
  275:     expect(transcript).toContain("collapseFinished: handoff?.collapseFinished ?? false");
  276:     expect(chatView).toContain("toolHandoffViewportQuiet.value = false;");
  277:     expect(chatView).toContain("if (suppressScrollCapture || toolHandoffViewportQuiet.value) return;");
  278:   });
  279: 
  280:   it("keeps handoff waiting inside the transient tool group", () => {
  281:     const transcript = read("src/components/chat/ChatTranscript.vue");
  282:     const toolWaitingIndex = transcript.indexOf("<div v-if=\"segment.showWaiting && isToolWaitingForResponse\" class=\"chat-transcript-tool-waiting-row\">");
  283:     const standaloneWaitingIndex = transcript.indexOf("<div v-else-if=\"segment.type === 'waiting'\" class=\"chat-transcript-thinking-block\">");
  284:     const toolGroupIndex = transcript.indexOf("v-else-if=\"segment.type === 'toolCalls'\"");
  285: 
  286:     expect(toolGroupIndex).toBeGreaterThan(-1);
  287:     expect(toolWaitingIndex).toBeGreaterThan(toolGroupIndex);
  288:     expect(toolWaitingIndex).toBeLessThan(standaloneWaitingIndex);
  289:     expect(transcript).toContain("'waiting-placeholder': isStandaloneWaitingPlaceholder");
  290:     expect(transcript).toContain("const isToolWaitingForResponse = computed(() => isWaitingForResponse.value && hasTransientToolCalls.value);");
  291:     expect(transcript).toContain("const isStandaloneWaitingPlaceholder = computed(() => isWaitingForResponse.value && !hasTransientToolCalls.value);");
  292:     expect(transcript).toContain("showWaiting: false,");
  293:     expect(transcript).toContain("lastToolSegment.showWaiting = true;");
  294:     expect(transcript).toContain("segment.showWaiting && isToolWaitingForResponse");
  295:     expect(transcript).toContain(".chat-transcript-tool-waiting-row {");
  296:     expect(transcript).not.toContain("'waiting-placeholder': isWaitingForResponse");
  297:   });
  298: 
  299:   it("sorts assistant segments by persisted render order", () => {
  300:     const transcript = read("src/components/chat/ChatTranscript.vue");
  301: 
  302:     expect(transcript).toContain("function renderPartsForMessage(item: MessageRenderItem): AssistantRenderPart[]");
  303:     expect(transcript).toContain("assertCanonicalRenderParts(item.message.renderParts, `message:${item.message.id}`);");
  304:     expect(transcript).toContain("synthesizeLegacyRenderParts(item.message, {");
  305:     expect(transcript).toContain("const canonicalLiveRenderParts = computed(() => {");
  306:     expect(transcript).toContain("props.liveRenderParts.length > 0");
  307:     expect(transcript).toContain("const hasVisibleActiveThinkingBlock = computed(() =>");
  308:     expect(transcript).toContain(":class=\"{ active: segment.active, 'is-clickable': true }\"");
  309:     expect(transcript).toContain("data-render-part-kind=\"toolCall\"");
  310:     expect(transcript).toContain("data-render-part-kind=\"text\"");
  311:     expect(transcript).not.toContain("splitToolCallsByRenderOrder");
  312:   });
  313: 
  314:   it("coalesces consecutive tool-only assistant rounds before rendering", () => {
  315:     const transcript = read("src/components/chat/ChatTranscript.vue");
  316:     const batches = read("src/composables/toolCallBatches.ts");
  317: 
  318:     expect(batches).toContain("let pendingToolOnlyItem: T | null = null;");
  319:     expect(batches).toContain("pendingToolOnlyItem ??= item;");
  320:     expect(batches).toContain("const displayToolCalls = pendingToolCalls.length > 0 ? [...pendingToolCalls] : undefined;");
  321:     expect(transcript).toContain("function historyRenderSegmentsForGroup(group: MessageGroup): HistoryRenderSegment[]");
  322:     expect(transcript).toContain("function historyToolSegmentKey(toolCalls: ToolCallDisplay[], fallbackId: string)");
  323:     expect(transcript).toContain("key: historyToolSegmentKey(pendingToolCalls, pendingToolPart.id),");
  324:     expect(transcript).toContain("pendingToolCalls.push(...segment.toolCalls);");
  325:     expect(transcript).toContain("const hasToolFilter = hasExplicitDisplayToolCalls(item) || !!toolCallInfosForMessage(item.message);");
  326:     expect(transcript).toContain(".filter((part) => part.kind !== \"toolCall\" || !hasToolFilter || visibleToolIds.has(part.toolCall.id))");
  327:     expect(transcript).toContain("hiddenToolCallsByItemId.set(item.id, toolCalls ?? []);");
  328:     expect(batches).toContain("const hasToolCallsProperty = Object.prototype.hasOwnProperty.call(item, \"toolCalls\");");
  329:     expect(transcript).toContain("'tool-only': isToolOnlyMessageGroup(group),");
  330:     expect(transcript).not.toContain("tool-only-followup");
  331:     expect(transcript).not.toContain("shouldTightenToolOnlyGap");
  332:   });
  333: 
  334:   it("attaches knowledge proposals only inside their assistant message group", () => {
  335:     const transcript = read("src/components/chat/ChatTranscript.vue");
  336: 
  337:     expect(transcript).toContain("for (const group of groups) {");
  338:     expect(transcript).toContain("if (group.role !== \"assistant\") continue;");
  339:     expect(transcript).toContain("const nextRequestTool = group.items.find(");
  340:     expect(transcript).toContain("const prevRequestTool = [...group.items].reverse().find(");
  341:   });
  342: });
  343: 
````

### src/__tests__/displaySettingsLayout.test.ts:1-280

````ts
    1: import { readFileSync } from "node:fs";
    2: import { resolve } from "node:path";
    3: import { describe, expect, it } from "vitest";
    4: 
    5: const cwd = process.cwd();
    6: 
    7: function read(relPath: string) {
    8:   return readFileSync(resolve(cwd, relPath), "utf8");
    9: }
   10: 
   11: describe("display settings transcript alignment", () => {
   12:   it("keeps main and Unity embed color styles separately configurable", () => {
   13:     const theme = read("src/composables/useTheme.ts");
   14:     const displayPanel = read("src/components/settings/DisplaySettings.vue");
   15:     const settingsState = read("src/composables/useSettingsState.ts");
   16:     const app = read("src/App.vue");
   17:     const html = read("index.html");
   18:     const zh = read("src/language/zh.json");
   19:     const en = read("src/language/en.json");
   20: 
   21:     expect(theme).toContain('export type ThemeScope = "main" | "unityEmbed";');
   22:     expect(theme).toContain('unityEmbed: "locus-unity-embed-theme-preference"');
   23:     expect(theme).toContain('main: "dark"');
   24:     expect(theme).toContain('unityEmbed: "dark"');
   25:     expect(theme).toContain("unityEmbedPreference");
   26:     expect(theme).toContain("setThemePreference(scope: ThemeScope, pref: ThemePreference)");
   27: 
   28:     expect(app).toContain('initTheme(isUnityEmbedWindow ? "unityEmbed" : "main")');
   29:     expect(html).toContain("locus-unity-embed-theme-preference");
   30:     expect(html).toContain("var fallback='dark';");
   31: 
   32:     expect(displayPanel).toContain("mainPreference");
   33:     expect(displayPanel).toContain("unityEmbedPreference");
   34:     expect(displayPanel).toContain("settings.display.themeMainWindow");
   35:     expect(displayPanel).toContain("settings.display.themeUnityEmbedWindow");
   36:     expect(displayPanel).toContain("setThemePreference('main', $event as ThemePreference)");
   37:     expect(displayPanel).toContain("setThemePreference('unityEmbed', $event as ThemePreference)");
   38:     expect(settingsState).toContain('setThemePreference("main", "dark");');
   39:     expect(settingsState).toContain('setThemePreference("unityEmbed", "dark");');
   40: 
   41:     expect(zh).toContain('"settings.display.themeMainWindow": "主窗口"');
   42:     expect(zh).toContain('"settings.display.themeUnityEmbedWindow": "Unity 嵌入窗口"');
   43:     expect(en).toContain('"settings.display.themeMainWindow": "Main Window"');
   44:     expect(en).toContain('"settings.display.themeUnityEmbedWindow": "Unity Embedded Window"');
   45:   });
   46: 
   47:   it("adds a session user message right-align toggle that defaults to on", () => {
   48:     const displaySettings = read("src/composables/useDisplaySettings.ts");
   49:     const displayPanel = read("src/components/settings/DisplaySettings.vue");
   50:     const transcript = read("src/components/chat/ChatTranscript.vue");
   51:     const zh = read("src/language/zh.json");
   52:     const en = read("src/language/en.json");
   53: 
   54:     expect(displaySettings).toContain("rightAlignUserMessages: boolean;");
   55:     expect(displaySettings).toContain("rightAlignUserMessages: true,");
   56: 
   57:     expect(displayPanel).toContain(":model-value=\"display.rightAlignUserMessages\"");
   58:     expect(displayPanel).toContain(":aria-label=\"t('settings.display.rightAlignUserMessages')\"");
   59:     expect(displayPanel).toContain("@update:model-value=\"setDisplay('rightAlignUserMessages', $event)\"");
   60:     expect(displayPanel).toContain("{{ t(\"settings.display.rightAlignUserMessages\") }}");
   61: 
   62:     expect(transcript).toContain("const { state: displaySettings } = useDisplaySettings();");
   63:     expect(transcript).toContain("function shouldRightAlignUserMessageGroup(group: Pick<MessageGroup, \"role\">) {");
   64:     expect(transcript).toContain("'user-align-right': shouldRightAlignUserMessageGroup(group),");
   65:     expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-role.is-session {");
   66:     expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-content.is-session {");
   67:     expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-item-stack.is-session {");
   68:     expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-plain-text {");
   69: 
   70:     expect(zh).toContain('"settings.display.rightAlignUserMessages": "会话窗口中将用户消息右对齐"');
   71:     expect(en).toContain('"settings.display.rightAlignUserMessages": "Right-align user messages in the session view"');
   72:   });
   73: 
   74:   it("adds a Git tree status icon merge toggle", () => {
   75:     const displaySettings = read("src/composables/useDisplaySettings.ts");
   76:     const displayPanel = read("src/components/settings/DisplaySettings.vue");
   77:     const stagingArea = read("src/components/collab/StagingArea.vue");
   78:     const commitDetail = read("src/components/collab/CommitDetail.vue");
   79:     const collabStyles = read("src/components/collab/collabPreview.css");
   80:     const zh = read("src/language/zh.json");
   81:     const en = read("src/language/en.json");
   82: 
   83:     expect(displaySettings).toContain("mergeGitTreeStatusIcon: boolean;");
   84:     expect(displaySettings).toContain("mergeGitTreeStatusIcon: true,");
   85: 
   86:     expect(displayPanel).toContain("settings.display.gitViewTitle");
   87:     expect(displayPanel).toContain(":model-value=\"display.mergeGitTreeStatusIcon\"");
   88:     expect(displayPanel).toContain(":aria-label=\"t('settings.display.mergeGitTreeStatusIcon')\"");
   89:     expect(displayPanel).toContain("@update:model-value=\"setDisplay('mergeGitTreeStatusIcon', $event)\"");
   90: 
   91:     for (const component of [stagingArea, commitDetail]) {
   92:       expect(component).toContain("const { state: displaySettings } = useDisplaySettings();");
   93:       expect(component).toContain("displaySettings.mergeGitTreeStatusIcon");
   94:       expect(component).toContain("fileTreeIconClasses(row.file)");
   95:       expect(component).toContain("staging-tree-status-spacer");
   96:     }
   97: 
   98:     expect(collabStyles).toContain(".staging-tree-file-icon.is-git-status-icon.status-modified");
   99:     expect(collabStyles).toContain("color: var(--git-status-modified);");
  100:     expect(collabStyles).toContain("color: var(--git-status-added);");
  101:     expect(collabStyles).toContain("color: var(--git-status-deleted);");
  102: 
  103:     expect(zh).toContain('"settings.display.mergeGitTreeStatusIcon": "层级视图用彩色图标显示修改状态"');
  104:     expect(en).toContain('"settings.display.mergeGitTreeStatusIcon": "Use colored icons for Git tree status"');
  105:   });
  106: 
  107:   it("adds a Git terminal suggestion visibility toggle that defaults to visible", () => {
  108:     const displaySettings = read("src/composables/useDisplaySettings.ts");
  109:     const displayPanel = read("src/components/settings/DisplaySettings.vue");
  110:     const gitTerminal = read("src/components/GitTerminal.vue");
  111:     const zh = read("src/language/zh.json");
  112:     const en = read("src/language/en.json");
  113: 
  114:     expect(displaySettings).toContain("hideGitCommandSuggestions: boolean;");
  115:     expect(displaySettings).toContain("hideGitCommandSuggestions: false,");
  116: 
  117:     expect(displayPanel).toContain(":model-value=\"display.hideGitCommandSuggestions\"");
  118:     expect(displayPanel).toContain(":aria-label=\"t('settings.display.hideGitCommandSuggestions')\"");
  119:     expect(displayPanel).toContain("@update:model-value=\"setDisplay('hideGitCommandSuggestions', $event)\"");
  120:     expect(displayPanel).toContain("{{ t(\"settings.display.hideGitCommandSuggestions\") }}");
  121: 
  122:     expect(gitTerminal).toContain('import { useDisplaySettings } from "../composables/useDisplaySettings";');
  123:     expect(gitTerminal).toContain("const { state: displaySettings } = useDisplaySettings();");
  124:     expect(gitTerminal).toContain("!displaySettings.hideGitCommandSuggestions && lines.length === 0");
  125: 
  126:     expect(zh).toContain('"settings.display.hideGitCommandSuggestions": "隐藏 Git 候选项"');
  127:     expect(en).toContain('"settings.display.hideGitCommandSuggestions": "Hide Git command suggestions"');
  128:   });
  129: 
  130:   it("adds a subagent completion notification toggle that defaults to off", () => {
  131:     const displaySettings = read("src/composables/useDisplaySettings.ts");
  132:     const displayPanel = read("src/components/settings/DisplaySettings.vue");
  133:     const notifications = read("src/services/systemNotifications.ts");
  134:     const bootstrap = read("src/composables/useAppBootstrap.ts");
  135:     const zh = read("src/language/zh.json");
  136:     const en = read("src/language/en.json");
  137: 
  138:     expect(displaySettings).toContain("notifyOnSubagentDone: boolean;");
  139:     expect(displaySettings).toContain("notifyOnSubagentDone: false,");
  140: 
  141:     expect(displayPanel).toContain(":model-value=\"display.notifyOnSubagentDone\"");
  142:     expect(displayPanel).toContain(":aria-label=\"t('settings.display.notifyOnSubagentDone')\"");
  143:     expect(displayPanel).toContain("@update:model-value=\"setDisplay('notifyOnSubagentDone', $event)\"");
  144:     expect(displayPanel).toContain("{{ t(\"settings.display.notifyOnSubagentDone\") }}");
  145: 
  146:     expect(notifications).toContain("context.isSubagent ? state.notifyOnSubagentDone : state.notifyOnChatDone");
  147:     expect(notifications).toContain('context.isSubagent ? "notifications.subagentDoneTitle" : "notifications.chatDoneTitle"');
  148:     expect(bootstrap).toContain("...(session?.parentSessionId ? { isSubagent: true } : {})");
  149: 
  150:     expect(zh).toContain('"settings.display.notifyOnSubagentDone": "Subagent 完成时通知"');
  151:     expect(zh).toContain('"notifications.subagentDoneTitle": "Subagent 已完成"');
  152:     expect(en).toContain('"settings.display.notifyOnSubagentDone": "Notify when a Sub-agent completes"');
  153:     expect(en).toContain('"notifications.subagentDoneTitle": "Sub-agent complete"');
  154:   });
  155: 
  156:   it("adds file diff review target settings that default to the current window", () => {
  157:     const displaySettings = read("src/composables/useDisplaySettings.ts");
  158:     const displayPanel = read("src/components/settings/DisplaySettings.vue");
  159:     const chatChangesPanel = read("src/components/ChatChangesPanel.vue");
  160:     const chatView = read("src/components/ChatView.vue");
  161:     const chatReviewWindow = read("src/components/ChatDiffReviewWindow.vue");
  162:     const fileDiffViewer = read("src/components/diff/FileDiffViewer.vue");
  163:     const collabView = read("src/components/CollabView.vue");
  164:     const app = read("src/App.vue");
  165:     const capability = read("src-tauri/capabilities/default.json");
  166:     const zh = read("src/language/zh.json");
  167:     const en = read("src/language/en.json");
  168: 
  169:     expect(displaySettings).toContain('export type DiffReviewTarget = "inline" | "window";');
  170:     expect(displaySettings).toContain("chatDiffReviewTarget: DiffReviewTarget;");
  171:     expect(displaySettings).toContain("gitDiffReviewTarget: DiffReviewTarget;");
  172:     expect(displaySettings).toContain('chatDiffReviewTarget: "inline",');
  173:     expect(displaySettings).toContain('gitDiffReviewTarget: "inline",');
  174: 
  175:     expect(displayPanel).toContain('<div class="section-label">{{ t("settings.display.diffReviewTitle") }}</div>');
  176:     expect(displayPanel).toContain('<p class="section-desc">{{ t("settings.display.diffReviewDesc") }}</p>');
  177:     expect(displayPanel).toContain("settings.display.diffReviewChatTarget");
  178:     expect(displayPanel).toContain("settings.display.diffReviewGitTarget");
  179:     expect(displayPanel).toContain(":model-value=\"display.chatDiffReviewTarget\"");
  180:     expect(displayPanel).toContain(":model-value=\"display.gitDiffReviewTarget\"");
  181:     expect(displayPanel).toContain("@update:model-value=\"setDisplay('chatDiffReviewTarget', $event as DiffReviewTarget)\"");
  182:     expect(displayPanel).toContain("@update:model-value=\"setDisplay('gitDiffReviewTarget', $event as DiffReviewTarget)\"");
  183:     expect(displayPanel.indexOf("settings.display.diffReviewTitle")).toBeGreaterThan(
  184:       displayPanel.indexOf("settings.display.panelBehaviorTitle"),
  185:     );
  186:     expect(displayPanel.indexOf("settings.display.diffReviewTitle")).toBeLessThan(
  187:       displayPanel.indexOf("settings.display.gitViewTitle"),
  188:     );
  189: 
  190:     expect(chatChangesPanel).toContain("displaySettings.chatDiffReviewTarget === \"window\"");
  191:     expect(chatChangesPanel).toContain("openChatDiffReviewWindow({ request })");
  192:     expect(collabView).toContain("displaySettings.gitDiffReviewTarget === \"window\"");
  193:     expect(collabView).toContain("openFileDiffReviewWindow({ request })");
  194:     expect(chatView).toContain("openInlineDiffInWindow");
  195:     expect(chatView).toContain("chat.changes.openReviewWindow");
  196:     expect(chatReviewWindow).toContain(":hide-text-display-controls=\"true\"");
  197:     expect(chatReviewWindow).toContain("fileDiffViewerRef?.hasTextDisplayModeControl");
  198:     expect(chatReviewWindow).toContain("fileDiffViewerRef.toggleTextDisplayMode()");
  199:     expect(chatReviewWindow.indexOf("common.openInEditor")).toBeLessThan(
  200:       chatReviewWindow.indexOf("diff.mode.sideBySide"),
  201:     );
  202:     expect(fileDiffViewer).toContain("hideTextDisplayControls?: boolean;");
  203:     expect(fileDiffViewer).toContain("hasTextDisplayModeControl");
  204:     expect(fileDiffViewer).toContain("toggleTextDisplayMode,");
  205: 
  206:     expect(app).toContain("isChatDiffReviewWindowLocation");
  207:     expect(app).toContain("<ChatDiffReviewWindow v-else-if=\"isChatDiffReviewWindow\" />");
  208:     expect(capability).toContain('"chat-diff-review"');
  209: 
  210:     expect(zh).toContain('"settings.display.panelBehaviorDesc": "控制会话面板的打开与关闭"');
  211:     expect(zh).toContain('"settings.display.diffReviewTitle": "文件修改审查"');
  212:     expect(zh).toContain('"settings.display.diffReviewDesc": "选择文件修改审查的默认打开位置"');
  213:     expect(zh).toContain('"settings.display.diffReviewChatTarget": "会话修改"');
  214:     expect(zh).toContain('"settings.display.diffReviewGitTarget": "Git 修改"');
  215:     expect(zh).toContain('"settings.display.diffReviewWindow": "独立窗口"');
  216:     expect(en).toContain('"settings.display.panelBehaviorDesc": "Control how session panels open and close"');
  217:     expect(en).toContain('"settings.display.diffReviewTitle": "File Change Review"');
  218:     expect(en).toContain('"settings.display.diffReviewDesc": "Choose where file change reviews open by default"');
  219:     expect(en).toContain('"settings.display.diffReviewChatTarget": "Session changes"');
  220:     expect(en).toContain('"settings.display.diffReviewGitTarget": "Git changes"');
  221:     expect(en).toContain('"settings.display.diffReviewWindow": "Separate window"');
  222:   });
  223: 
  224:   it("adds a completed thinking block visibility toggle that defaults to hidden", () => {
  225:     const displaySettings = read("src/composables/useDisplaySettings.ts");
  226:     const displayPanel = read("src/components/settings/DisplaySettings.vue");
  227:     const transcript = read("src/components/chat/ChatTranscript.vue");
  228:     const zh = read("src/language/zh.json");
  229:     const en = read("src/language/en.json");
  230: 
  231:     expect(displaySettings).toContain("hideThinkingBlocks: boolean;");
  232:     expect(displaySettings).toContain("hideThinkingBlocks: true,");
  233: 
  234:     expect(displayPanel).toContain(":model-value=\"display.hideThinkingBlocks\"");
  235:     expect(displayPanel).toContain(":aria-label=\"t('settings.display.hideThinkingBlocks')\"");
  236:     expect(displayPanel).toContain("@update:model-value=\"setDisplay('hideThinkingBlocks', $event)\"");
  237:     expect(displayPanel).toContain("{{ t(\"settings.display.hideThinkingBlocks\") }}");
  238: 
  239:     expect(transcript).toContain("function shouldHideThinkingBlocks()");
  240:     expect(transcript).toContain("return displaySettings.hideThinkingBlocks !== false;");
  241:     expect(transcript).toContain("return !shouldHideThinkingBlocks() && !!item.message.thinkingContent?.trim();");
  242:     expect(transcript).toContain("const hasVisibleCompletedThinkingContent = computed(() =>");
  243:     expect(transcript).toContain("&& canonicalLiveRenderParts.value.some((part) =>");
  244:     expect(transcript).toContain("const hasVisibleActiveThinkingBlock = computed(() =>");
  245:     expect(transcript).toContain("part.kind === \"thinking\" && part.active");
  246:     expect(transcript).toContain("hasVisibleActiveThinkingBlock.value || hasVisibleCompletedThinkingContent.value");
  247:     expect(transcript).toContain("hasThinkingContent: hasVisibleCompletedThinkingContent.value,");
  248:     expect(transcript).toContain("function shouldRenderTransientThinkingSegment(");
  249:     expect(transcript).toContain("return !!part.active || (!shouldHideThinkingBlocks() && part.content.trim().length > 0);");
  250:     expect(transcript).toMatch(/if \(part\.kind === "thinking"\) \{\s+if \(!shouldRenderTransientThinkingSegment\(part\)\) continue;\s+flushPendingTools\(\);/);
  251: 
  252:     expect(zh).toContain('"settings.display.hideThinkingBlocks": "隐藏已完成思考块"');
  253:     expect(en).toContain('"settings.display.hideThinkingBlocks": "Hide completed thinking blocks"');
  254:   });
  255: });
  256: 
````

## 补充附录：Markdown 正文渲染链路

这部分用于补齐普通消息正文与工具输出的最终 Markdown 渲染链路。工具折叠本身在 `ToolCallCollection` / `ToolCallBlock`，但工具输出内容会进入 `MarkdownRenderer`。

### src/components/MarkdownRenderer.vue

行数：631

````vue

<script setup lang="ts">
import { computed } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";
import hljs from "../hljs";
import { renderHighlightedCodeLines } from "../composables/markdownCodeLines";
import { normalizeExternalMarkdownHref } from "../composables/markdownExternalLinks";
import { injectAssetRefs, injectFileRefs, injectWorkspaceMentions } from "../composables/markdownInject";
import { normalizeMarkdownForRender } from "../composables/markdownRender";
import { wrapMarkdownTables } from "../composables/markdownTableHtml";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";

const props = defineProps<{
  content: string;
  cursor?: boolean;
  enableFileRefs?: boolean;
  highlightTerms?: string[];
}>();

function escapeHtml(source: string): string {
  return source
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function escapeRegExp(source: string): string {
  return source.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function normalizeHighlightTerms(terms?: string[]): string[] {
  if (!terms?.length) return [];
  return [...new Set(
    terms
      .map((value) => value.trim())
      .filter(Boolean),
  )].sort((left, right) => right.length - left.length);
}

function shouldSkipHighlight(node: Text): boolean {
  let current: HTMLElement | null = node.parentElement;
  while (current) {
    const tagName = current.tagName;
    if (
      tagName === "PRE"
      || tagName === "SCRIPT"
      || tagName === "STYLE"
      || tagName === "TEXTAREA"
    ) {
      return true;
    }
    if (tagName === "MARK" && current.classList.contains("markdown-search-mark")) {
      return true;
    }
    current = current.parentElement;
  }
  return false;
}

function highlightHtml(html: string, terms: string[]): string {
  if (!html || !terms.length || typeof DOMParser === "undefined") return html;
  const regex = new RegExp(`(${terms.map(escapeRegExp).join("|")})`, "gi");
  const parser = new DOMParser();
  const doc = parser.parseFromString(`<body>${html}</body>`, "text/html");
  const root = doc.body;
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (!(node instanceof Text)) return NodeFilter.FILTER_REJECT;
      if (!node.nodeValue?.trim()) return NodeFilter.FILTER_REJECT;
      if (shouldSkipHighlight(node)) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });

  const textNodes: Text[] = [];
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (node instanceof Text) textNodes.push(node);
  }

  for (const textNode of textNodes) {
    const value = textNode.nodeValue ?? "";
    regex.lastIndex = 0;
    let match: RegExpExecArray | null;
    let lastIndex = 0;
    let hasMatch = false;
    const fragment = doc.createDocumentFragment();
    while ((match = regex.exec(value)) !== null) {
      hasMatch = true;
      if (match.index > lastIndex) {
        fragment.append(doc.createTextNode(value.slice(lastIndex, match.index)));
      }
      const mark = doc.createElement("mark");
      mark.className = "markdown-search-mark";
      mark.textContent = match[0];
      fragment.append(mark);
      lastIndex = match.index + match[0].length;
      if (match[0].length === 0) {
        regex.lastIndex += 1;
      }
    }
    if (!hasMatch) continue;
    if (lastIndex < value.length) {
      fragment.append(doc.createTextNode(value.slice(lastIndex)));
    }
    textNode.parentNode?.replaceChild(fragment, textNode);
  }

  return root.innerHTML;
}

const md = new Marked(
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code: string, lang: string) {
      const normalizedLang = lang.trim().toLowerCase();
      if (normalizedLang === "tree") {
        return renderHighlightedCodeLines(escapeHtml(code), false);
      }

      let highlighted = escapeHtml(code);
      if (normalizedLang && hljs.getLanguage(normalizedLang)) {
        highlighted = hljs.highlight(code, { language: normalizedLang }).value;
      }
      return renderHighlightedCodeLines(highlighted);
    },
  }),
  {
    breaks: true,
    gfm: true,
    hooks: {
      postprocess(html) {
        return wrapMarkdownTables(html);
      },
    },
  }
);

const renderedHtml = computed(() => {
  if (!props.content) return "";
  try {
    let html = md.parse(normalizeMarkdownForRender(props.content)) as string;
    html = injectAssetRefs(html);
    html = injectWorkspaceMentions(html);
    if (props.enableFileRefs) {
      html = injectFileRefs(html);
    }
    if (props.cursor) {
      html = html.replace(
        /((?:\s*<\/[^>]+>)+\s*)$/,
        '<span class="streaming-cursor">▍</span>$1'
      );
    }
    const highlightTerms = normalizeHighlightTerms(props.highlightTerms);
    if (highlightTerms.length) {
      html = highlightHtml(html, highlightTerms);
    }
    return html;
  } catch {
    return props.content;
  }
});

function isHandledMarkdownMouseButton(event: MouseEvent): boolean {
  return event.button === 0 || event.button === 1;
}

async function openMarkdownHref(href: string): Promise<void> {
  try {
    await openUrl(href);
  } catch (error) {
    console.warn("Failed to open markdown link externally:", error);
    if (!hasTauriWindowRuntime()) {
      window.open(href, "_blank", "noopener,noreferrer");
    }
  }
}

function handleMarkdownLinkActivation(event: MouseEvent) {
  if (event.defaultPrevented || !isHandledMarkdownMouseButton(event)) return;
  if (!(event.target instanceof Element)) return;

  const anchor = event.target.closest("a[href]") as HTMLAnchorElement | null;
  if (!anchor) return;

  event.preventDefault();
  event.stopPropagation();

  const href = normalizeExternalMarkdownHref(anchor.getAttribute("href"));
  if (!href) return;

  void openMarkdownHref(href);
}
</script>

<template>
  <div
    class="markdown-body ui-select-text"
    @click="handleMarkdownLinkActivation"
    @auxclick="handleMarkdownLinkActivation"
    v-html="renderedHtml"
  />
</template>

<style>
.markdown-body {
  font-family: var(--font-prose);
  font-size: 14px;
  line-height: 1.68;
  word-break: break-word;
  color: var(--text-color);
  text-rendering: optimizeLegibility;
}

.markdown-body h1,
.markdown-body h2,
.markdown-body h3,
.markdown-body h4,
.markdown-body h5,
.markdown-body h6 {
  margin: 24px 0 10px;
  font-weight: 600;
  line-height: 1.35;
  letter-spacing: -0.01em;
}

.markdown-body > :first-child {
  margin-top: 0;
}

.markdown-body > :last-child {
  margin-bottom: 0;
}

.markdown-body h1 {
  font-size: 1.58em;
  margin-bottom: 14px;
}

.markdown-body h2 {
  font-size: 1.3em;
  padding-bottom: 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
}

.markdown-body h3 {
  font-size: 1.12em;
}

.markdown-body h4,
.markdown-body h5,
.markdown-body h6 {
  font-size: 1em;
  color: var(--text-secondary);
}

.markdown-body p,
.markdown-body ul,
.markdown-body ol,
.markdown-body blockquote,
.markdown-body hr,
.markdown-body pre,
.markdown-body .md-table-wrap {
  margin: 0 0 12px;
}

.markdown-body ul,
.markdown-body ol {
  padding-left: 20px;
}

.markdown-body li {
  margin: 4px 0;
}

.markdown-body li > ul,
.markdown-body li > ol {
  margin-top: 6px;
  margin-bottom: 6px;
}

.markdown-body ul li::marker {
  color: color-mix(in srgb, var(--text-secondary) 72%, transparent);
}

.markdown-body ol li::marker {
  color: var(--text-secondary);
  font-weight: 600;
}

.markdown-body blockquote {
  padding: 8px 12px;
  border-left: 2px solid color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 44%, transparent);
  border-radius: 0 6px 6px 0;
}

.markdown-body blockquote > :last-child {
  margin-bottom: 0;
}

.markdown-body a {
  color: var(--accent-color);
  text-decoration-line: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 0.16em;
  text-decoration-color: color-mix(in srgb, var(--accent-color) 40%, transparent);
  transition: color 0.15s ease, text-decoration-color 0.15s ease;
}

.markdown-body a:hover {
  text-decoration-color: currentColor;
}

.markdown-body hr {
  border: none;
  border-top: 1px solid var(--border-color);
  opacity: 0.8;
}

.markdown-body .md-table-wrap {
  width: fit-content;
  max-width: 100%;
  box-sizing: border-box;
  overflow-x: auto;
  overflow-y: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.markdown-body table {
  width: max-content;
  min-width: 100%;
  margin: 0;
  border-collapse: separate;
  border-spacing: 0;
  table-layout: auto;
  font-size: 13px;
  background: transparent;
}

.markdown-body th,
.markdown-body td {
  min-width: 120px;
  padding: 7px 10px;
  text-align: left;
  vertical-align: top;
  white-space: normal;
  overflow-wrap: anywhere;
  word-break: normal;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent) !important;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent) !important;
  color: var(--text-color) !important;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%) !important;
}

.markdown-body th {
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 68%, var(--panel-bg) 32%) !important;
  font-weight: 600;
  color: var(--text-secondary) !important;
}

.markdown-body tr:last-child td {
  border-bottom: none;
}

.markdown-body th:last-child,
.markdown-body td:last-child {
  border-right: none;
}

.markdown-body tbody tr:nth-child(even) td {
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--hover-bg) 18%) !important;
}

.markdown-body code {
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  padding: 1px 6px;
  border-radius: 4px;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  color: color-mix(in srgb, var(--text-color) 92%, var(--accent-color) 8%);
}

.markdown-body pre {
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg, var(--panel-bg)) 76%, transparent);
  overflow-x: auto;
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--panel-bg) 32%, transparent);
}

.markdown-body pre code {
  display: block;
  font-family: var(--font-mono-block);
  padding: 10px 0;
  background: transparent;
  font-size: 13px;
  line-height: 1.55;
  white-space: pre;
  overflow-x: auto;
  counter-reset: line;
  border: none;
  color: inherit;
}

.markdown-body pre code .code-line {
  display: grid;
  grid-template-columns: 46px minmax(0, 1fr);
  align-items: start;
  min-width: 100%;
}

.markdown-body pre code .code-line-tree {
  grid-template-columns: minmax(0, 1fr);
}

.markdown-body pre code .line-number {
  display: block;
  padding: 0 10px 0 0;
  text-align: right;
  color: color-mix(in srgb, var(--text-secondary) 78%, transparent);
  user-select: none;
  opacity: 0.5;
  font-size: 11px;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.markdown-body pre code .line-content {
  display: block;
  padding: 0 14px 0 12px;
  min-width: 0;
}

.markdown-body pre code .code-line-tree .line-content {
  padding-left: 14px;
}

.markdown-body img {
  max-width: 100%;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  cursor: pointer;
}

.markdown-body strong {
  font-weight: 600;
}

.markdown-body em {
  color: color-mix(in srgb, var(--text-color) 82%, var(--text-secondary) 18%);
}

.markdown-body mark.markdown-search-mark {
  padding: 0 2px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--accent-color) 22%, var(--hover-bg));
  color: inherit;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 12%, transparent);
}

.markdown-body mark.markdown-search-mark-target {
  background: color-mix(in srgb, var(--accent-color) 34%, var(--hover-bg));
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 28%, transparent),
    0 0 0 1px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.markdown-body :not(pre) > code a,
.markdown-body :not(pre) > code {
  text-decoration: none;
}

.md-asset-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 7px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  cursor: pointer;
  font-size: 0.88em;
  line-height: 1.5;
  vertical-align: baseline;
  font-weight: 500;
  color: var(--text-secondary);
}

.md-asset-chip:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.md-asset-chip-icon {
  font-size: 10px;
  opacity: 0.58;
}

.md-file-ref {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 22px;
  padding: 1px 6px 1px 5px;
  box-sizing: border-box;
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  cursor: pointer;
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  line-height: 18px;
  vertical-align: -2px;
  font-weight: 400;
  color: color-mix(in srgb, var(--text-color) 90%, var(--text-secondary) 10%);
}

.md-unity-asset-ref,
.md-unity-scene-object-ref {
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 54%, transparent);
  border-color: color-mix(in srgb, var(--border-color) 78%, transparent);
  color: color-mix(in srgb, var(--text-color) 90%, var(--text-secondary) 10%);
}

.md-workspace-ref {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 22px;
  padding: 1px 6px 1px 5px;
  box-sizing: border-box;
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  cursor: pointer;
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  line-height: 18px;
  vertical-align: -2px;
  font-weight: 400;
  color: color-mix(in srgb, var(--text-color) 86%, var(--text-secondary) 14%);
}

.md-file-ref:hover,
.md-file-ref:active {
  background: color-mix(in srgb, var(--hover-bg) 78%, var(--sidebar-bg, var(--hover-bg)) 22%);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.md-unity-asset-ref:hover,
.md-unity-asset-ref:active,
.md-unity-scene-object-ref:hover,
.md-unity-scene-object-ref:active {
  background: color-mix(in srgb, var(--accent-color) 5%, var(--hover-bg) 95%);
  border-color: color-mix(in srgb, var(--accent-color) 18%, var(--border-strong) 82%);
}

.md-workspace-ref:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.md-workspace-ref-prefix {
  margin-right: 1px;
  opacity: 0.58;
}

.md-workspace-ref-icon {
  margin-right: 2px;
}

.md-ref-label {
  min-width: 0;
  display: block;
  line-height: 18px;
}

.md-ref-icon {
  display: block;
  width: 14px;
  min-width: 14px;
  height: 14px;
  align-self: center;
  flex-shrink: 0;
  object-fit: contain;
  max-width: none;
  border: none;
  border-radius: 0;
  background: transparent;
  opacity: 0.82;
  cursor: inherit;
  pointer-events: none;
  user-select: none;
}

.md-ref-icon-lucide {
  display: block;
  opacity: 0.95;
  filter: none;
}

img.md-ref-icon-image {
  display: none;
}

.md-workspace-ref-prefix {
  display: none;
}

.streaming-cursor {
  color: var(--accent-color);
  font-weight: 400;
  margin-left: 1px;
  animation: streaming-cursor-blink 0.8s step-end infinite;
}

@keyframes streaming-cursor-blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0; }
}
</style>

````

### src/composables/markdownRender.ts

行数：34

````ts
const BLOCKQUOTE_PREFIX_RE = /^(\s*(?:>\s*)+)/;
const PUNCTUATION_TERMINATED_STRONG_RE =
  /((?:\*\*[^*\n]*[：:；;，,。.!！？?、）】》」』]\*\*)|(?:__[^_\n]*[：:；;，,。.!！？?、）】》」』]__))(?=[\p{L}\p{N}\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\[(（【「『<])/gu;

function blockquotePrefix(line: string): string | null {
  const match = line.match(BLOCKQUOTE_PREFIX_RE);
  return match?.[1]?.trimEnd() || null;
}

function normalizeLooseBlockquotes(markdown: string): string {
  const lines = markdown.split("\n");
  for (let index = 1; index < lines.length - 1; index += 1) {
    if (lines[index].trim() !== "") continue;
    if (lines[index - 1].trim() === "" || lines[index + 1].trim() === "") continue;

    const previousPrefix = blockquotePrefix(lines[index - 1]);
    const nextPrefix = blockquotePrefix(lines[index + 1]);
    if (!previousPrefix || !nextPrefix) continue;

    lines[index] = previousPrefix;
  }
  return lines.join("\n");
}

function normalizeStrongLabelSpacing(markdown: string): string {
  return markdown.replace(PUNCTUATION_TERMINATED_STRONG_RE, "$1 ");
}

export function normalizeMarkdownForRender(markdown: string): string {
  if (!markdown) return "";
  const normalizedLineEndings = markdown.replace(/\r\n/g, "\n");
  return normalizeStrongLabelSpacing(normalizeLooseBlockquotes(normalizedLineEndings));
}

````

### src/composables/markdownExternalLinks.ts

行数：18

````ts
const EXTERNAL_MARKDOWN_LINK_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

export function normalizeExternalMarkdownHref(rawHref: string | null | undefined): string | null {
  const href = rawHref?.trim();
  if (!href || href.startsWith("#")) return null;

  if (href.startsWith("//")) {
    return `https:${href}`;
  }

  try {
    const url = new URL(href);
    return EXTERNAL_MARKDOWN_LINK_PROTOCOLS.has(url.protocol) ? url.href : null;
  } catch {
    return null;
  }
}

````

### src/composables/markdownCodeLines.ts

行数：98

````ts
type OpenHtmlTag = {
  name: string;
  html: string;
};

const HTML_TOKEN_RE = /<\/?[a-z][^>]*>|\n|[^<\n]+|./gi;
const OPEN_TAG_RE = /^<([a-z][\w:-]*)(?:\s[^>]*)?>$/i;
const CLOSE_TAG_RE = /^<\/([a-z][\w:-]*)>$/i;
const SELF_CLOSING_TAG_RE = /^<([a-z][\w:-]*)(?:\s[^>]*)?\/>$/i;
const VOID_TAGS = new Set([
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
]);

function closingHtmlForOpenTags(openTags: OpenHtmlTag[]): string {
  return openTags
    .slice()
    .reverse()
    .map((tag) => `</${tag.name}>`)
    .join("");
}

function openingHtmlForOpenTags(openTags: OpenHtmlTag[]): string {
  return openTags.map((tag) => tag.html).join("");
}

function popClosingTag(openTags: OpenHtmlTag[], tagName: string): void {
  const normalized = tagName.toLowerCase();
  for (let i = openTags.length - 1; i >= 0; i -= 1) {
    if (openTags[i].name === normalized) {
      openTags.splice(i, 1);
      return;
    }
  }
}

export function splitHighlightedHtmlLines(source: string): string[] {
  const tokens = source.match(HTML_TOKEN_RE) ?? [];
  const lines: string[] = [];
  const openTags: OpenHtmlTag[] = [];
  let currentLine = "";

  for (const token of tokens) {
    if (token === "\n") {
      lines.push(currentLine + closingHtmlForOpenTags(openTags));
      currentLine = openingHtmlForOpenTags(openTags);
      continue;
    }

    currentLine += token;

    const closeMatch = token.match(CLOSE_TAG_RE);
    if (closeMatch) {
      popClosingTag(openTags, closeMatch[1]);
      continue;
    }

    if (SELF_CLOSING_TAG_RE.test(token)) {
      continue;
    }

    const openMatch = token.match(OPEN_TAG_RE);
    if (openMatch) {
      const name = openMatch[1].toLowerCase();
      if (!VOID_TAGS.has(name)) {
        openTags.push({ name, html: token });
      }
    }
  }

  lines.push(currentLine + closingHtmlForOpenTags(openTags));
  return lines;
}

export function renderHighlightedCodeLines(source: string, showLineNumbers = true): string {
  const lines = splitHighlightedHtmlLines(source);
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines
    .map((line, i) => (
      showLineNumbers
        ? `<span class="code-line"><span class="line-number">${i + 1}</span><span class="line-content">${line || " "}</span></span>`
        : `<span class="code-line code-line-tree"><span class="line-content">${line || " "}</span></span>`
    ))
    .join("\n");
}

````

### src/composables/markdownTableHtml.ts

行数：13

````ts
const TABLE_WRAPPER_CLASS = "md-table-wrap";
const WRAPPED_TABLE_RE = /<div class="md-table-wrap">\s*(<table\b[\s\S]*?<\/table>)\s*<\/div>/gi;
const TABLE_RE = /<table\b[\s\S]*?<\/table>/gi;

export function wrapMarkdownTables(html: string): string {
  if (!/<table\b/i.test(html)) return html;

  const normalizedHtml = html.replace(WRAPPED_TABLE_RE, "$1");
  return normalizedHtml.replace(TABLE_RE, (tableHtml) => (
    `<div class="${TABLE_WRAPPER_CLASS}">${tableHtml}</div>`
  ));
}

````

### src/__tests__/markdownRenderNormalization.test.ts

行数：54

````ts
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { Marked } from "marked";
import { normalizeMarkdownForRender } from "../composables/markdownRender";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("markdownRender normalization", () => {
  it("normalizes loose blockquotes before MarkdownRenderer parses them", () => {
    const source = read("src/components/MarkdownRenderer.vue");
    const markdown = [
      "> 受击打断",
      "",
      "> 强调动作游戏的主动反应",
      "",
      "> BOSS也应该对玩家的动作进行响应",
    ].join("\n");
    const html = new Marked({ breaks: true, gfm: true }).parse(
      normalizeMarkdownForRender(markdown),
    ) as string;

    expect(source).toContain('md.parse(normalizeMarkdownForRender(props.content))');
    expect(normalizeMarkdownForRender(markdown)).toBe([
      "> 受击打断",
      ">",
      "> 强调动作游戏的主动反应",
      ">",
      "> BOSS也应该对玩家的动作进行响应",
    ].join("\n"));
    expect(html).toBe([
      "<blockquote>",
      "<p>受击打断</p>",
      "<p>强调动作游戏的主动反应</p>",
      "<p>BOSS也应该对玩家的动作进行响应</p>",
      "</blockquote>",
      "",
    ].join("\n"));
  });

  it("adds a parsing boundary after punctuation-terminated bold labels", () => {
    const markdown = "> **特色：**强交互受击打断、高机动性（只有Top-Down能做）";
    const normalized = normalizeMarkdownForRender(markdown);
    const html = new Marked({ breaks: true, gfm: true }).parse(normalized) as string;

    expect(normalized).toBe("> **特色：** 强交互受击打断、高机动性（只有Top-Down能做）");
    expect(html).toContain("<strong>特色：</strong> 强交互受击打断、高机动性（只有Top-Down能做）");
  });
});

````

### src/__tests__/markdownTableStyles.test.ts

行数：41

````ts
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Markdown table styles", () => {
  it("wraps rendered tables and constrains cell styling in MarkdownRenderer", () => {
    const source = read("src/components/MarkdownRenderer.vue");

    expect(source).toContain('wrapMarkdownTables(html)');
    expect(source).toMatch(/\.markdown-body \.md-table-wrap\s*\{[\s\S]*width:\s*fit-content;[\s\S]*overflow-x:\s*auto;/);
    expect(source).toMatch(/\.markdown-body table\s*\{[\s\S]*width:\s*max-content;[\s\S]*min-width:\s*100%;/);
    expect(source).toMatch(/\.markdown-body th,\s*[\s\S]*\.markdown-body td\s*\{[\s\S]*overflow-wrap:\s*anywhere;[\s\S]*border-right:[\s\S]*!important;[\s\S]*border-bottom:[\s\S]*!important;/);
    expect(source).toMatch(/tbody tr:nth-child\(even\) td\s*\{[\s\S]*!important;/);
  });

  it("supports optional search-term highlights in MarkdownRenderer", () => {
    const source = read("src/components/MarkdownRenderer.vue");

    expect(source).toContain("highlightTerms?: string[];");
    expect(source).toContain("function normalizeHighlightTerms(terms?: string[]): string[] {");
    expect(source).toContain('mark.className = "markdown-search-mark";');
    expect(source).toContain("html = highlightHtml(html, highlightTerms);");
    expect(source).toMatch(/\.markdown-body mark\.markdown-search-mark\s*\{[\s\S]*border-radius:\s*4px;[\s\S]*background:[\s\S]*box-shadow:/);
    expect(source).toMatch(/\.markdown-body mark\.markdown-search-mark-target\s*\{[\s\S]*background:[\s\S]*box-shadow:/);
  });

  it("keeps editor table cells readable under the same theme constraints", () => {
    const source = read("src/components/ui/BaseMarkdownEditor.vue");

    expect(source).toMatch(/\.base-markdown-editor :deep\(\.vditor-reset table\)\s*\{[\s\S]*width:\s*max-content;[\s\S]*min-width:\s*100%;/);
    expect(source).toMatch(/\.base-markdown-editor :deep\(\.vditor-reset th\),\s*[\s\S]*\.base-markdown-editor :deep\(\.vditor-reset td\)\s*\{[\s\S]*overflow-wrap:\s*anywhere;[\s\S]*border-right:[\s\S]*!important;[\s\S]*border-bottom:[\s\S]*!important;/);
    expect(source).toMatch(/tbody tr:nth-child\(even\) td\)\s*\{[\s\S]*!important;/);
  });
});

````

### src/__tests__/markdownExternalLinks.test.ts

行数：35

````ts
import { describe, expect, it } from "vitest";
import { normalizeExternalMarkdownHref } from "../composables/markdownExternalLinks";

describe("markdownExternalLinks", () => {
  it("keeps explicit web links for external opening", () => {
    expect(normalizeExternalMarkdownHref("https://github.com/yasirkula/UnityRuntimeInspector")).toBe(
      "https://github.com/yasirkula/UnityRuntimeInspector",
    );
    expect(normalizeExternalMarkdownHref(" http://example.com/docs ")).toBe(
      "http://example.com/docs",
    );
  });

  it("normalizes protocol-relative web links to https", () => {
    expect(normalizeExternalMarkdownHref("//github.com/org/repo")).toBe(
      "https://github.com/org/repo",
    );
  });

  it("allows external app protocols handled by the OS", () => {
    expect(normalizeExternalMarkdownHref("mailto:team@example.com")).toBe(
      "mailto:team@example.com",
    );
    expect(normalizeExternalMarkdownHref("tel:+15551234567")).toBe("tel:+15551234567");
  });

  it("blocks internal, relative, and unsafe hrefs from WebView navigation", () => {
    expect(normalizeExternalMarkdownHref("#section")).toBeNull();
    expect(normalizeExternalMarkdownHref("/docs/intro")).toBeNull();
    expect(normalizeExternalMarkdownHref("docs/intro.md")).toBeNull();
    expect(normalizeExternalMarkdownHref("javascript:alert(1)")).toBeNull();
    expect(normalizeExternalMarkdownHref("file:///C:/Windows/System32/drivers/etc/hosts")).toBeNull();
  });
});

````

### src/__tests__/markdownCodeLines.test.ts

行数：72

````ts
import { describe, expect, it } from "vitest";
import { parseFragment } from "parse5";
import hljs from "../hljs";
import { renderHighlightedCodeLines, splitHighlightedHtmlLines } from "../composables/markdownCodeLines";

type ParseNode = {
  nodeName: string;
  tagName?: string;
  attrs?: Array<{ name: string; value: string }>;
  childNodes?: ParseNode[];
  value?: string;
};

function classNames(node: ParseNode): string[] {
  const classAttr = node.attrs?.find((attr) => attr.name === "class")?.value ?? "";
  return classAttr.split(/\s+/).filter(Boolean);
}

function hasClass(node: ParseNode, className: string): boolean {
  return classNames(node).includes(className);
}

function textContent(node: ParseNode | undefined): string {
  if (!node) return "";
  if (node.nodeName === "#text") return node.value ?? "";
  return (node.childNodes ?? []).map((child) => textContent(child)).join("");
}

function childByClass(node: ParseNode, className: string): ParseNode | undefined {
  return (node.childNodes ?? []).find((child) => hasClass(child, className));
}

describe("markdown code line rendering", () => {
  it("keeps code lines as siblings when highlight.js spans cross line breaks", () => {
    const code = `public static class Screenshot

{
    [FlutterBridgeMessageDoc("截取当前画面", Kind = FlutterBridgeMessageKind.Query,
        ParamsType = typeof(ScreenshotCaptureResult))]
    public const string Capture = "Screenshot.Capture";
}`;
    const highlighted = hljs.highlight(code, { language: "csharp" }).value;

    expect(highlighted).toContain('class="hljs-meta"');
    expect(highlighted).toContain('FlutterBridgeMessageKind.Query,\n        ParamsType');

    const rendered = renderHighlightedCodeLines(highlighted);
    const fragment = parseFragment(`<code>${rendered}</code>`) as ParseNode;
    const codeElement = fragment.childNodes?.find((child) => child.tagName === "code");
    const directCodeLines = codeElement?.childNodes?.filter((child) => hasClass(child, "code-line")) ?? [];

    expect(directCodeLines).toHaveLength(7);
    expect(directCodeLines.map((line) => textContent(childByClass(line, "line-number")).trim())).toEqual([
      "1",
      "2",
      "3",
      "4",
      "5",
      "6",
      "7",
    ]);
    expect(textContent(childByClass(directCodeLines[4], "line-content"))).toContain("ParamsType");
  });

  it("reopens active highlight spans on the next rendered line", () => {
    expect(splitHighlightedHtmlLines('<span class="hljs-meta">A\nB</span>')).toEqual([
      '<span class="hljs-meta">A</span>',
      '<span class="hljs-meta">B</span>',
    ]);
  });
});

````
