import type { ChatMessage, ToolCallDisplay, ToolCallInfo } from "../types";

const INTERRUPTED_TOOL_RESULT = "工具执行被用户中止，未返回结果。";

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
  toolCalls?: ToolCallInfo[];
  attachedKnowledgeProposalCount?: number;
  isKnowledgeProposal?: boolean;
}

export type AssistantToolMergeResult<T> = T & {
  displayToolCalls?: ToolCallInfo[];
};

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

export function filterToolCallsByActiveIds(
  toolCalls: ToolCallInfo[] | undefined,
  activeIds: Set<string>,
): ToolCallInfo[] | undefined {
  if (!toolCalls || toolCalls.length === 0) return undefined;
  if (activeIds.size === 0) return [...toolCalls];

  const filtered = toolCalls.filter((toolCall) => !activeIds.has(toolCall.id));
  return filtered.length > 0 ? filtered : undefined;
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
  let pendingToolOnlyItems: T[] = [];
  let pendingToolCalls: ToolCallInfo[] = [];

  const flushPendingToolOnlyItems = () => {
    if (pendingToolOnlyItems.length === 0) return;
    for (const pendingItem of pendingToolOnlyItems) {
      merged.push({
        ...pendingItem,
        displayToolCalls: pendingItem.toolCalls ? [...pendingItem.toolCalls] : undefined,
      });
    }
    pendingToolOnlyItems = [];
    pendingToolCalls = [];
  };

  for (const item of items) {
    const currentToolCalls = item.toolCalls ?? [];
    const hasResponseText = !item.isKnowledgeProposal && item.content.trim().length > 0;
    const isToolOnlyRound =
      !item.isKnowledgeProposal
      && !hasResponseText
      && currentToolCalls.length > 0;
    const canAbsorbPendingRounds = !item.isKnowledgeProposal && hasResponseText;

    if (isToolOnlyRound) {
      pendingToolOnlyItems.push(item);
      pendingToolCalls.push(...currentToolCalls);
      continue;
    }

    if (pendingToolCalls.length > 0 && canAbsorbPendingRounds) {
      merged.push({
        ...item,
        displayToolCalls: [...pendingToolCalls, ...currentToolCalls],
      });
      pendingToolOnlyItems = [];
      pendingToolCalls = [];
      continue;
    }

    flushPendingToolOnlyItems();

    merged.push({
      ...item,
      displayToolCalls: currentToolCalls.length > 0 ? [...currentToolCalls] : undefined,
    });
  }

  flushPendingToolOnlyItems();

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
      && runningCount === 0
      && errorCount === 0
      && interruptedCount === 0,
  };
}
