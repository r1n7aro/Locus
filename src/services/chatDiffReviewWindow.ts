import type { FileDiffPayload, FileDiffRequest } from "../types";
import type { WorkspaceRef } from "./project";
import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const CHAT_DIFF_REVIEW_WINDOW_LABEL = "chat-diff-review";
export const CHAT_DIFF_REVIEW_WINDOW_PATH = "/chat-diff-review";
export const CHAT_DIFF_REVIEW_WINDOW_EVENT = "chat-diff-review:payload";
export const CHAT_DIFF_REVIEW_WINDOW_FLAG = "chatDiffReview";
export const CHAT_DIFF_REVIEW_WINDOW_TITLE = "Locus File Review";

export interface ChatDiffReviewWindowPayload {
  request?: FileDiffRequest;
  payload?: FileDiffPayload;
  diffKey?: string;
  workspaceRef?: WorkspaceRef | null;
}

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

function isFileDiffRequest(value: unknown): value is FileDiffRequest {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<FileDiffRequest>;
  return typeof candidate.source === "string"
    && typeof candidate.filePath === "string"
    && typeof candidate.detail === "string";
}

function parseRequestParam(raw: string | null): FileDiffRequest | undefined {
  if (!raw) return undefined;
  try {
    const parsed = JSON.parse(raw);
    return isFileDiffRequest(parsed) ? parsed : undefined;
  } catch {
    return undefined;
  }
}

export function isChatDiffReviewWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === CHAT_DIFF_REVIEW_WINDOW_PATH
    || locationLike.search.includes(`${CHAT_DIFF_REVIEW_WINDOW_FLAG}=1`);
}

export function getChatDiffReviewWindowPayload(
  search = window.location.search,
): ChatDiffReviewWindowPayload {
  const params = new URLSearchParams(search);
  const checkoutId = trimOrEmpty(params.get("checkoutId"));
  const generationRaw = params.get("workspaceGeneration");
  const expectedGeneration = generationRaw && /^\d+$/.test(generationRaw)
    ? Number(generationRaw)
    : null;
  return {
    request: parseRequestParam(params.get("request")),
    diffKey: trimOrEmpty(params.get("diffKey")),
    workspaceRef: checkoutId && Number.isSafeInteger(expectedGeneration)
      ? { checkoutId, expectedGeneration }
      : null,
  };
}

export function buildChatDiffReviewWindowQuery(
  payload: ChatDiffReviewWindowPayload,
): string {
  const params = new URLSearchParams({
    [CHAT_DIFF_REVIEW_WINDOW_FLAG]: "1",
  });
  if (payload.request) {
    params.set("request", JSON.stringify(payload.request));
  } else if (payload.diffKey?.trim()) {
    params.set("diffKey", payload.diffKey.trim());
  } else if (payload.payload?.key.trim()) {
    params.set("diffKey", payload.payload.key.trim());
  }
  if (
    payload.workspaceRef?.checkoutId.trim()
    && Number.isSafeInteger(payload.workspaceRef.expectedGeneration)
  ) {
    params.set("checkoutId", payload.workspaceRef.checkoutId.trim());
    params.set("workspaceGeneration", String(payload.workspaceRef.expectedGeneration));
  }
  return params.toString();
}

export function buildChatDiffReviewWindowUrl(
  payload: ChatDiffReviewWindowPayload,
): string {
  return buildSubWindowUrl(buildChatDiffReviewWindowQuery(payload));
}

function eventPayload(input: ChatDiffReviewWindowPayload): ChatDiffReviewWindowPayload {
  const scoped = input.workspaceRef ? { workspaceRef: input.workspaceRef } : {};
  if (input.payload) {
    return { payload: input.payload, diffKey: input.payload.key, ...scoped };
  }
  if (input.request) return { request: input.request, ...scoped };
  return { diffKey: trimOrEmpty(input.diffKey), ...scoped };
}

export async function openChatDiffReviewWindow(
  input: ChatDiffReviewWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const payload = eventPayload(input);
  const result = await openSubWindow({
    kind: CHAT_DIFF_REVIEW_WINDOW_LABEL,
    title: CHAT_DIFF_REVIEW_WINDOW_TITLE,
    width: 1180,
    height: 760,
    minWidth: 760,
    minHeight: 520,
    resizable: true,
    maximizable: true,
    minimizable: false,
  }, buildChatDiffReviewWindowQuery(payload));
  if (result.existing) {
    await result.window?.emit(CHAT_DIFF_REVIEW_WINDOW_EVENT, payload);
  }
  return true;
}

export const openFileDiffReviewWindow = openChatDiffReviewWindow;
