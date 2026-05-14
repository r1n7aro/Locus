import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { FileDiffPayload, FileDiffRequest } from "../types";
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
  return {
    request: parseRequestParam(params.get("request")),
    diffKey: trimOrEmpty(params.get("diffKey")),
  };
}

export function buildChatDiffReviewWindowUrl(
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
  return `${CHAT_DIFF_REVIEW_WINDOW_PATH}?${params.toString()}`;
}

function eventPayload(input: ChatDiffReviewWindowPayload): ChatDiffReviewWindowPayload {
  if (input.payload) return { payload: input.payload, diffKey: input.payload.key };
  if (input.request) return { request: input.request };
  return { diffKey: trimOrEmpty(input.diffKey) };
}

export async function openChatDiffReviewWindow(
  input: ChatDiffReviewWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const payload = eventPayload(input);
  const existingWindow = await WebviewWindow.getByLabel(CHAT_DIFF_REVIEW_WINDOW_LABEL);
  if (existingWindow) {
    await existingWindow.emit(CHAT_DIFF_REVIEW_WINDOW_EVENT, payload);
    await existingWindow.setFocus();
    return true;
  }

  await new Promise<void>((resolve, reject) => {
    const reviewWindow = new WebviewWindow(CHAT_DIFF_REVIEW_WINDOW_LABEL, {
      url: buildChatDiffReviewWindowUrl(payload),
      title: CHAT_DIFF_REVIEW_WINDOW_TITLE,
      width: 1180,
      height: 760,
      minWidth: 760,
      minHeight: 520,
      decorations: false,
      resizable: true,
      closable: true,
      minimizable: false,
      maximizable: true,
      parent: getCurrentWebviewWindow(),
      center: true,
      shadow: true,
    });

    reviewWindow.once("tauri://created", () => {
      resolve();
    });
    reviewWindow.once("tauri://error", (event) => {
      reject(event);
    });
  });

  return true;
}

export const openFileDiffReviewWindow = openChatDiffReviewWindow;
