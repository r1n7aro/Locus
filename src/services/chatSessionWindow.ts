import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const CHAT_SESSION_WINDOW_EVENT = "chat-session-window:payload";
export const CHAT_SESSION_WINDOW_FLAG = "chatSessionWindow";
export const CHAT_SESSION_WINDOW_PATH = "/chat-session-window";

export interface ChatSessionWindowPayload {
  sessionId: string;
  title?: string;
  newChat?: boolean;
}

let newChatWindowSequence = 0;

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

function hashSessionId(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

export function chatSessionWindowKind(sessionId: string): string {
  const normalized = trimOrEmpty(sessionId);
  const readable = normalized
    .replace(/[^a-zA-Z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 36) || "session";
  return `chat-session-${readable}-${hashSessionId(normalized)}`;
}

export function newChatSessionWindowKind(): string {
  newChatWindowSequence += 1;
  const timestamp = Date.now().toString(36);
  const sequence = newChatWindowSequence.toString(36);
  return `chat-session-new-${timestamp}-${sequence}`;
}

export function isChatSessionWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === CHAT_SESSION_WINDOW_PATH
    || locationLike.search.includes(`${CHAT_SESSION_WINDOW_FLAG}=1`);
}

export function getChatSessionWindowPayload(
  search = window.location.search,
): ChatSessionWindowPayload {
  const params = new URLSearchParams(search);
  return {
    sessionId: trimOrEmpty(params.get("sessionId")),
    title: trimOrEmpty(params.get("title")) || undefined,
    newChat: params.get("newChat") === "1",
  };
}

export function buildChatSessionWindowQuery(payload: ChatSessionWindowPayload): string {
  const params = new URLSearchParams({
    [CHAT_SESSION_WINDOW_FLAG]: "1",
    sessionId: trimOrEmpty(payload.sessionId),
  });
  if (payload.title?.trim()) {
    params.set("title", payload.title.trim());
  }
  if (payload.newChat) {
    params.set("newChat", "1");
  }
  return params.toString();
}

export function buildChatSessionWindowUrl(payload: ChatSessionWindowPayload): string {
  return buildSubWindowUrl(buildChatSessionWindowQuery(payload));
}

export async function openChatSessionWindow(
  payload: ChatSessionWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const sessionId = trimOrEmpty(payload.sessionId);
  if (!sessionId) return false;
  const title = trimOrEmpty(payload.title) || sessionId;
  const normalizedPayload = { sessionId, title };
  const result = await openSubWindow({
    kind: chatSessionWindowKind(sessionId),
    title: `Locus - ${title}`,
    width: 1040,
    height: 780,
    minWidth: 660,
    minHeight: 480,
    resizable: true,
    maximizable: true,
    minimizable: true,
  }, buildChatSessionWindowQuery(normalizedPayload));

  if (result.existing) {
    await result.window?.emit(CHAT_SESSION_WINDOW_EVENT, normalizedPayload);
  }
  return true;
}

export async function openNewChatSessionWindow(title = "New session"): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const normalizedTitle = trimOrEmpty(title) || "New session";
  const payload: ChatSessionWindowPayload = {
    sessionId: "",
    title: normalizedTitle,
    newChat: true,
  };
  await openSubWindow({
    kind: newChatSessionWindowKind(),
    title: `Locus - ${normalizedTitle}`,
    width: 1040,
    height: 780,
    minWidth: 660,
    minHeight: 480,
    resizable: true,
    maximizable: true,
    minimizable: true,
  }, buildChatSessionWindowQuery(payload));
  return true;
}
