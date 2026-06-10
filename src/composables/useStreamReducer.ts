import { hydrateChatMessageIntent, parseUserIntentMeta } from "./chatInputIntents";
import { sortedAssistantRenderParts } from "./assistantRenderParts";
import { resolveToolCallDisplayShape } from "./toolCallBatches";
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
  | { type: "setCompacting"; value: boolean };

export function buildToolResultMessages(
  activeToolCalls: ToolCallDisplay[],
  createdAt = Date.now() / 1000,
): ChatMessage[] {
  return activeToolCalls
    .filter((toolCall) => toolCall.output !== undefined)
    .map((toolCall): ChatMessage => {
      const message: ChatMessage = {
        id: `tool_result_${toolCall.id}`,
        role: "tool",
        content: toolCall.output ?? "",
        createdAt,
        toolCallId: toolCall.id,
      };
      if (toolCall.images && toolCall.images.length > 0) {
        message.images = toolCall.images;
      }
      return message;
    });
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

export function isPendingUserMessageId(id: string): boolean {
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

export function isMatchingPendingUserMessage(candidate: ChatMessage, message: ChatMessage): boolean {
  if (candidate.role !== "user" || !isPendingUserMessageId(candidate.id)) return false;
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
      const rawName = event.toolName || currentPart?.toolCall.name || existing?.name || "";
      const rawArguments = event.arguments || currentPart?.toolCall.arguments || existing?.arguments || "";
      const displayShape = resolveToolCallDisplayShape({
        name: rawName,
        arguments: rawArguments,
      });
      mutations.push({
        type: "upsertLiveRenderPart",
        part: {
          kind: "toolCall",
          id: liveOrder.id,
          order: liveOrder.order,
          toolCall: {
            ...(currentPart?.toolCall ?? {
              id: event.toolCallId,
              name: displayShape.name,
              arguments: "",
            }),
            id: event.toolCallId,
            name: displayShape.name,
            arguments: displayShape.arguments,
            order: liveOrder.order.seq,
          },
        },
      });
      if (existing) {
        const updates: Partial<ToolCallDisplay> = {};
        if (displayShape.name && displayShape.name !== existing.name) {
          updates.name = displayShape.name;
        }
        if (displayShape.arguments && displayShape.arguments !== existing.arguments) {
          updates.arguments = displayShape.arguments;
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
          toolCall: {
            id: event.toolCallId,
            name: displayShape.name,
            arguments: displayShape.arguments,
            status: "running",
            order: liveOrder.order.seq,
          },
        });
      }
      break;
    }

    case "toolCallDone": {
      const updates: Partial<ToolCallDisplay> = {
        status: event.outcome,
        output: event.output,
        progress: null,
      };
      if (event.images && event.images.length > 0) {
        updates.images = event.images;
      }
      mutations.push({
        type: "updateLiveToolPart",
        toolCallId: event.toolCallId,
        updates: { outcome: event.outcome, recordedOutput: event.output },
      });
      mutations.push({
        type: "updateToolCall",
        id: event.toolCallId,
        updates,
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
        const rawName = event.toolName || existingNested?.name || "";
        const rawArguments = event.arguments || existingNested?.arguments || "";
        const displayShape = resolveToolCallDisplayShape({
          name: rawName,
          arguments: rawArguments,
        });
        if (existingNested) {
          const updates: Partial<ToolCallDisplay> = {};
          if (displayShape.name && displayShape.name !== existingNested.name) {
            updates.name = displayShape.name;
          }
          if (displayShape.arguments && displayShape.arguments !== existingNested.arguments) {
            updates.arguments = displayShape.arguments;
          }
          if (Object.keys(updates).length > 0) {
            mutations.push({ type: "updateNestedToolCall", parentId: event.parentToolCallId, childId: event.toolCallId, updates });
          }
        } else {
          const order = markToolOrder(undefined, event.order);
          mutations.push({
            type: "addNestedToolCall",
            parentId: event.parentToolCallId,
            toolCall: {
              id: event.toolCallId,
              name: displayShape.name,
              arguments: displayShape.arguments,
              status: "running",
              order,
            },
          });
        }
      }
      break;
    }

    case "subagentToolCallDone": {
      const updates: Partial<ToolCallDisplay> = {
        status: event.outcome,
        output: event.output,
      };
      if (event.images && event.images.length > 0) {
        updates.images = event.images;
      }
      mutations.push({
        type: "updateNestedToolCall",
        parentId: event.parentToolCallId,
        childId: event.toolCallId,
        updates,
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
