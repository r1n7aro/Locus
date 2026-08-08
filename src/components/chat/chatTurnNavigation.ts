import type { ChatMessage } from "../../types";
import { displayUserMessageContent } from "../../composables/chatUserMessageDisplay";

export interface ChatTurnNavigationItem {
  id: string;
  prompt: string;
  response: string;
}

export interface ChatTurnNavigationAnchor {
  id: string;
  start: number;
}

function previewText(value: string | null | undefined) {
  return (value ?? "").replace(/\r\n?/g, "\n").trim();
}

/** Build one navigation item for every user turn. The first following
 * assistant message supplies the secondary preview shown in the tooltip. */
export function buildChatTurnNavigationItems(
  messages: readonly ChatMessage[],
): ChatTurnNavigationItem[] {
  const items: ChatTurnNavigationItem[] = [];

  for (let index = 0; index < messages.length; index += 1) {
    const message = messages[index];
    if (!message || message.role !== "user") continue;

    let response = "";
    for (let responseIndex = index + 1; responseIndex < messages.length; responseIndex += 1) {
      const candidate = messages[responseIndex];
      if (!candidate || candidate.role === "tool") continue;
      if (candidate.role === "user") break;
      if (candidate.role === "assistant") {
        response = previewText(candidate.content);
        if (response) break;
      }
    }

    items.push({
      id: message.id,
      prompt: previewText(displayUserMessageContent(message.content)),
      response,
    });
  }

  return items;
}

/** Return every turn range intersecting the viewport. Keeping all intersecting
 * ranges active mirrors Codex when two short turns are visible together. */
export function findActiveChatTurnIds(
  anchors: readonly ChatTurnNavigationAnchor[],
  viewportTop: number,
  viewportBottom: number,
  contentEnd: number,
) {
  if (anchors.length === 0) return [];

  const activeIds: string[] = [];
  for (let index = 0; index < anchors.length; index += 1) {
    const anchor = anchors[index]!;
    const nextStart = anchors[index + 1]?.start ?? contentEnd;
    if (nextStart > viewportTop && anchor.start < viewportBottom) {
      activeIds.push(anchor.id);
    }
  }

  if (activeIds.length > 0) return activeIds;

  let fallback = anchors[0]!;
  for (const anchor of anchors) {
    if (anchor.start > viewportTop) break;
    fallback = anchor;
  }
  return [fallback.id];
}
