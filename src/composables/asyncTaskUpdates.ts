import type {
  AssistantRenderPart,
  AsyncTaskUpdatedEvent,
  ChatMessage,
  NestedToolCallUpdatedEvent,
  ToolCallDisplay,
  ToolCallInfo,
} from "../types";

export function asyncTaskDisplayStatus(
  status: AsyncTaskUpdatedEvent["status"],
): ToolCallDisplay["status"] {
  if (status === "completed") return "done";
  if (status === "failed") return "error";
  if (status === "cancelled") return "interrupted";
  return "running";
}

function updateToolCallInfo(
  toolCall: ToolCallInfo,
  update: AsyncTaskUpdatedEvent,
  outcome: ToolCallInfo["outcome"],
): [ToolCallInfo, boolean] {
  if (toolCall.id === update.toolCallId) {
    const next = {
      ...toolCall,
      recordedOutput: update.output,
    };
    if (outcome) {
      next.outcome = outcome;
    } else {
      delete next.outcome;
    }
    return [next, true];
  }
  if (!toolCall.nestedToolCalls?.length) return [toolCall, false];
  let nestedChanged = false;
  const nestedToolCalls = toolCall.nestedToolCalls.map((nested) => {
    const [next, changed] = updateToolCallInfo(nested, update, outcome);
    nestedChanged ||= changed;
    return next;
  });
  return nestedChanged
    ? [{ ...toolCall, nestedToolCalls }, true]
    : [toolCall, false];
}

function updateRenderPart(
  part: AssistantRenderPart,
  update: AsyncTaskUpdatedEvent,
  outcome: ToolCallInfo["outcome"],
): [AssistantRenderPart, boolean] {
  if (part.kind !== "toolCall") return [part, false];
  const [toolCall, changed] = updateToolCallInfo(part.toolCall, update, outcome);
  return changed ? [{ ...part, toolCall }, true] : [part, false];
}

export function applyAsyncTaskUpdateToMessages(
  messages: ChatMessage[],
  update: AsyncTaskUpdatedEvent,
): ChatMessage[] {
  const displayStatus = asyncTaskDisplayStatus(update.status);
  const outcome: ToolCallInfo["outcome"] = displayStatus === "running"
    ? undefined
    : displayStatus;

  return messages.map((message) => {
    if (message.id !== update.assistantMessageId) return message;
    let changed = false;
    const toolCalls = message.toolCalls?.map((toolCall) => {
      const [next, didChange] = updateToolCallInfo(toolCall, update, outcome);
      changed ||= didChange;
      return next;
    });
    const renderParts = message.renderParts?.map((part) => {
      const [next, didChange] = updateRenderPart(part, update, outcome);
      changed ||= didChange;
      return next;
    });
    return changed ? { ...message, toolCalls, renderParts } : message;
  });
}

function upsertNestedToolCall(
  toolCall: ToolCallInfo,
  update: NestedToolCallUpdatedEvent,
): [ToolCallInfo, boolean] {
  if (toolCall.id !== update.toolCallId) return [toolCall, false];
  const nested = toolCall.nestedToolCalls ?? [];
  const index = nested.findIndex((call) => call.id === update.nestedToolCall.id);
  const nestedToolCalls = index < 0
    ? [...nested, update.nestedToolCall]
    : nested.map((call, position) => (position === index ? { ...call, ...update.nestedToolCall } : call));
  return [{ ...toolCall, nestedToolCalls }, true];
}

/**
 * A background Claude subagent works after its Agent card's run ended: each
 * of its calls arrives as a persisted record, added to or replacing the
 * card's nested call with the same id.
 */
export function applyNestedToolCallUpdateToMessages(
  messages: ChatMessage[],
  update: NestedToolCallUpdatedEvent,
): ChatMessage[] {
  return messages.map((message) => {
    if (message.id !== update.assistantMessageId) return message;
    let changed = false;
    const toolCalls = message.toolCalls?.map((toolCall) => {
      const [next, didChange] = upsertNestedToolCall(toolCall, update);
      changed ||= didChange;
      return next;
    });
    const renderParts = message.renderParts?.map((part) => {
      if (part.kind !== "toolCall") return part;
      const [toolCall, didChange] = upsertNestedToolCall(part.toolCall, update);
      changed ||= didChange;
      return didChange ? { ...part, toolCall } : part;
    });
    return changed ? { ...message, toolCalls, renderParts } : message;
  });
}
