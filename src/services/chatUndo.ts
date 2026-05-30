import type { ChatMessage } from "../types";

export function findUndoRestoreUserMessage(
  messages: ChatMessage[],
  assistantMessageId: string,
): ChatMessage | null {
  const assistantIndex = messages.findIndex((message) => message.id === assistantMessageId);
  if (assistantIndex <= 0) return null;

  for (let index = assistantIndex - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === "user") {
      return message;
    }
  }

  return null;
}

export function findUndoRestoreUserText(
  messages: ChatMessage[],
  assistantMessageId: string,
): string | null {
  return findUndoRestoreUserMessage(messages, assistantMessageId)?.content ?? null;
}
