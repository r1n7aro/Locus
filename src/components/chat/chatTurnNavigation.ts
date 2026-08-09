import type { ChatMessage, SessionTurnPreview } from "../../types";
import { displayUserMessageContent } from "../../composables/chatUserMessageDisplay";

export interface ChatTurnNavigationItem {
  id: string;
  prompt: string;
  response: string;
  deferred?: boolean;
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
  indexedUserMessageIds: readonly string[] = [],
): ChatTurnNavigationItem[] {
  const loadedItems: ChatTurnNavigationItem[] = [];

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

    loadedItems.push({
      id: message.id,
      prompt: previewText(displayUserMessageContent(message.content)),
      response,
    });
  }

  if (indexedUserMessageIds.length === 0) return loadedItems;

  const loadedById = new Map(loadedItems.map((item) => [item.id, item]));
  const orderedIds = [...indexedUserMessageIds];
  const indexedIds = new Set(orderedIds);
  for (const item of loadedItems) {
    if (indexedIds.has(item.id)) continue;
    indexedIds.add(item.id);
    orderedIds.push(item.id);
  }
  return orderedIds.map((id) => loadedById.get(id) ?? {
    id,
    prompt: "",
    response: "",
    deferred: true,
  });
}

export function normalizeChatTurnPreview(preview: SessionTurnPreview): ChatTurnNavigationItem {
  return {
    id: preview.messageId,
    prompt: previewText(displayUserMessageContent(preview.prompt)),
    response: previewText(preview.response),
  };
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
