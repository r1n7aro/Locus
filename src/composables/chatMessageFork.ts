import type { ChatMessage } from "../types";

export function isMessageFromActiveStream(
  message: ChatMessage | null | undefined,
  activeRunId: string | null | undefined,
  isStreaming: boolean,
): boolean {
  if (!message || message.role !== "assistant" || !isStreaming || !activeRunId) {
    return false;
  }

  return message.renderParts?.some((part) => part.order.runId === activeRunId) ?? false;
}
