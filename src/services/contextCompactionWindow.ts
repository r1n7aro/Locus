import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const CONTEXT_COMPACTION_WINDOW_LABEL = "context-compaction";
export const CONTEXT_COMPACTION_WINDOW_PATH = "/context-compaction";
export const CONTEXT_COMPACTION_WINDOW_EVENT = "context-compaction:payload";
export const CONTEXT_COMPACTION_WINDOW_FLAG = "contextCompaction";
export const CONTEXT_COMPACTION_WINDOW_TITLE = "Locus Compacted Context";

export interface ContextCompactionWindowPayload {
  sessionId: string;
  messageId: string;
}

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

export function isContextCompactionWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === CONTEXT_COMPACTION_WINDOW_PATH
    || locationLike.search.includes(`${CONTEXT_COMPACTION_WINDOW_FLAG}=1`);
}

export function getContextCompactionWindowPayload(
  search = window.location.search,
): ContextCompactionWindowPayload {
  const params = new URLSearchParams(search);
  return {
    sessionId: trimOrEmpty(params.get("sessionId")),
    messageId: trimOrEmpty(params.get("messageId")),
  };
}

export function buildContextCompactionWindowQuery(
  payload: ContextCompactionWindowPayload,
): string {
  return new URLSearchParams({
    [CONTEXT_COMPACTION_WINDOW_FLAG]: "1",
    sessionId: payload.sessionId.trim(),
    messageId: payload.messageId.trim(),
  }).toString();
}

export function buildContextCompactionWindowUrl(
  payload: ContextCompactionWindowPayload,
): string {
  return buildSubWindowUrl(buildContextCompactionWindowQuery(payload));
}

export async function openContextCompactionWindow(
  payload: ContextCompactionWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;
  if (!payload.sessionId.trim() || !payload.messageId.trim()) return false;

  const normalizedPayload = {
    sessionId: payload.sessionId.trim(),
    messageId: payload.messageId.trim(),
  };
  const result = await openSubWindow({
    kind: CONTEXT_COMPACTION_WINDOW_LABEL,
    title: CONTEXT_COMPACTION_WINDOW_TITLE,
    width: 920,
    height: 720,
    minWidth: 600,
    minHeight: 420,
    resizable: true,
    maximizable: true,
    minimizable: false,
  }, buildContextCompactionWindowQuery(normalizedPayload));
  if (result.existing) {
    await result.window?.emit(CONTEXT_COMPACTION_WINDOW_EVENT, normalizedPayload);
  }
  return true;
}
