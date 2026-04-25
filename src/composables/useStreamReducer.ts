import type { StreamEvent, ChatMessage, TokenUsage, TodoItem, ToolCallDisplay, PendingQuestion, PendingToolConfirm } from "../types";

export interface StreamState {
  messages: ChatMessage[];
  streamingText: string;
  rawStreamText: string;
  streamingThinking: string;
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
  | { type: "setThinking"; value: boolean; startTime?: number }
  | { type: "updateThinkingDuration"; duration: number }
  | { type: "addToolCall"; toolCall: ToolCallDisplay }
  | { type: "updateToolCall"; id: string; updates: Partial<ToolCallDisplay> }
  | { type: "addNestedToolCall"; parentId: string; toolCall: ToolCallDisplay }
  | { type: "updateNestedToolCall"; parentId: string; childId: string; updates: Partial<ToolCallDisplay> }
  | { type: "appendToolDelta"; id: string; delta: string }
  | { type: "pushMessage"; message: ChatMessage }
  | { type: "upsertMessage"; message: ChatMessage }
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

export function reduceStreamEvent(state: StreamState, event: StreamEvent): StreamMutation[] {
  const mutations: StreamMutation[] = [];

  // Note: auto-reactivation of streaming removed — streaming is now controlled
  // exclusively by explicit sendChat/cancelChat actions. Late events from a
  // cancelled run are filtered by runId in the chat store.

  switch (event.type) {
    case "textDelta":
      mutations.push({ type: "appendRawText", text: event.text });
      if (state.isThinking && state.thinkingStartTime > 0) {
        mutations.push({ type: "updateThinkingDuration", duration: Math.round((Date.now() - state.thinkingStartTime) / 1000) });
      }
      mutations.push({ type: "setThinking", value: false });
      break;

    case "thinkingDelta":
      mutations.push({ type: "appendThinking", text: event.text });
      if (!state.isThinking) {
        mutations.push({ type: "setThinking", value: true, startTime: Date.now() });
      }
      break;

    case "toolCallStart": {
      const existing = state.activeToolCalls.find((t) => t.id === event.toolCallId);
      if (existing) {
        if (event.arguments) {
          mutations.push({ type: "updateToolCall", id: event.toolCallId, updates: { arguments: event.arguments } });
        }
      } else {
        mutations.push({
          type: "addToolCall",
          toolCall: { id: event.toolCallId, name: event.toolName, arguments: event.arguments, status: "running" },
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

    case "cancelled":
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
      break;

    case "error":
      mutations.push({ type: "resetRound" });
      mutations.push({ type: "clearPendingInputs" });
      mutations.push({ type: "setStreaming", value: false });
      break;
  }

  return mutations;
}
