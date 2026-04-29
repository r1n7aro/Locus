import { hydrateChatMessageIntent } from "./chatInputIntents";
import type { StreamEvent, ChatMessage, TokenUsage, TodoItem, ToolCallDisplay, PendingQuestion, PendingToolConfirm, ImageAttachment } from "../types";

export interface StreamState {
  messages: ChatMessage[];
  streamingText: string;
  rawStreamText: string;
  streamingThinking: string;
  streamSequence: number;
  streamingTextOrder: number;
  thinkingOrder: number;
  isStreaming: boolean;
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
  | { type: "setThinking"; value: boolean; startTime?: number }
  | { type: "updateThinkingDuration"; duration: number }
  | { type: "addToolCall"; toolCall: ToolCallDisplay }
  | { type: "updateToolCall"; id: string; updates: Partial<ToolCallDisplay> }
  | { type: "addNestedToolCall"; parentId: string; toolCall: ToolCallDisplay }
  | { type: "updateNestedToolCall"; parentId: string; childId: string; updates: Partial<ToolCallDisplay> }
  | { type: "appendToolDelta"; id: string; delta: string }
  | { type: "pushMessage"; message: ChatMessage }
  | { type: "upsertMessage"; message: ChatMessage }
  | { type: "upsertUserMessage"; message: ChatMessage }
  | { type: "replaceMessages"; messages: ChatMessage[] }
  | { type: "pushToolResults" }
  | { type: "resetRound" }
  | { type: "clearPendingInputs" }
  | { type: "clearPendingInput"; questionId: string }
  | { type: "updateUsage"; usage: TokenUsage }
  | { type: "setQuestion"; question: PendingQuestion | null }
  | { type: "enqueueToolConfirm"; confirm: PendingToolConfirm }
  | { type: "addUndoable"; messageId: string }
  | { type: "setTodos"; runId: string; todos: TodoItem[] }
  | { type: "setStreaming"; value: boolean }
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

function pendingUserMessageId(id: string): boolean {
  return id.startsWith("user_pending_") || id.startsWith("embedded_user_");
}

function imageFingerprint(images: ImageAttachment[] | undefined): string {
  return (images ?? [])
    .map((image) => `${image.mimeType}\u{0}${image.data}`)
    .join("\u{1}");
}

function isMatchingPendingUserMessage(candidate: ChatMessage, message: ChatMessage): boolean {
  if (candidate.role !== "user" || !pendingUserMessageId(candidate.id)) return false;
  if (imageFingerprint(candidate.images) !== imageFingerprint(message.images)) return false;
  if (candidate.content === message.content) return true;
  if (candidate.thinkingSignature && candidate.thinkingSignature === message.thinkingSignature) return true;
  return Math.abs(candidate.createdAt - message.createdAt) <= 60;
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

export function reduceStreamEvent(state: StreamState, event: StreamEvent): StreamMutation[] {
  const mutations: StreamMutation[] = [];

  const nextStreamOrder = () => state.streamSequence + 1;

  const markStreamSequence = (order: number) => {
    if (order > state.streamSequence) {
      mutations.push({ type: "setStreamSequence", value: order });
    }
  };

  const markTextOrder = () => {
    if (state.streamingTextOrder > 0 || state.rawStreamText.length > 0) return;
    const order = nextStreamOrder();
    mutations.push({ type: "setStreamingTextOrder", order });
    markStreamSequence(order);
  };

  const markThinkingOrder = () => {
    if (state.thinkingOrder > 0 || state.streamingThinking.length > 0) return;
    const order = nextStreamOrder();
    mutations.push({ type: "setThinkingOrder", order });
    markStreamSequence(order);
  };

  const markToolOrder = (existing?: ToolCallDisplay) => {
    if (existing?.order && existing.order > 0) return existing.order;
    const order = nextStreamOrder();
    markStreamSequence(order);
    return order;
  };

  const finishThinkingBeforeTools = () => {
    if (state.isThinking && state.thinkingStartTime > 0) {
      mutations.push({ type: "updateThinkingDuration", duration: Math.round((Date.now() - state.thinkingStartTime) / 1000) });
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
      markTextOrder();
      mutations.push({ type: "appendRawText", text: event.text });
      if (state.isThinking && state.thinkingStartTime > 0) {
        mutations.push({ type: "updateThinkingDuration", duration: Math.round((Date.now() - state.thinkingStartTime) / 1000) });
      }
      mutations.push({ type: "setThinking", value: false });
      break;

    case "thinkingDelta":
      markThinkingOrder();
      mutations.push({ type: "appendThinking", text: event.text });
      if (!state.isThinking) {
        mutations.push({ type: "setThinking", value: true, startTime: Date.now() });
      }
      break;

    case "toolCallStart": {
      finishThinkingBeforeTools();
      const existing = state.activeToolCalls.find((t) => t.id === event.toolCallId);
      if (existing) {
        const updates: Partial<ToolCallDisplay> = {};
        if (event.arguments) {
          updates.arguments = event.arguments;
        }
        if (!existing.order || existing.order <= 0) {
          updates.order = markToolOrder(existing);
        }
        if (Object.keys(updates).length > 0) {
          mutations.push({ type: "updateToolCall", id: event.toolCallId, updates });
        }
      } else {
        const order = markToolOrder();
        mutations.push({
          type: "addToolCall",
          toolCall: { id: event.toolCallId, name: event.toolName, arguments: event.arguments, status: "running", order },
        });
      }
      break;
    }

    case "toolCallDone": {
      mutations.push({
        type: "updateToolCall",
        id: event.toolCallId,
        updates: { status: event.outcome, output: event.output },
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

    case "subagentToolCallStart": {
      const parentTc = state.activeToolCalls.find((t) => t.id === event.parentToolCallId);
      if (parentTc) {
        const existingNested = parentTc.nestedToolCalls?.find((t) => t.id === event.toolCallId);
        if (existingNested) {
          if (event.arguments) {
            mutations.push({ type: "updateNestedToolCall", parentId: event.parentToolCallId, childId: event.toolCallId, updates: { arguments: event.arguments } });
          }
        } else {
          mutations.push({
            type: "addNestedToolCall",
            parentId: event.parentToolCallId,
            toolCall: { id: event.toolCallId, name: event.toolName, arguments: event.arguments, status: "running" },
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
        },
      });
      mutations.push({ type: "pushToolResults" });
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

    case "compactDone":
      mutations.push({ type: "replaceMessages", messages: event.messages });
      if (state.tokenUsage.contextTokens > 0) {
        mutations.push({
          type: "updateUsage",
          usage: {
            ...state.tokenUsage,
            contextTokens: 0,
          },
        });
      }
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
      if (event.fullText) {
        const existingMessage = state.messages.find((message) => message.id === event.messageId);
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
          },
        });
      }
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
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
          },
        });
      }
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
      break;
    }

    case "error":
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
      break;
  }

  return mutations;
}
