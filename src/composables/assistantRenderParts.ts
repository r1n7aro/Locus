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

