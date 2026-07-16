import type { LexicalRebuildStatus } from "../types";
import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_LABEL = "knowledge-lexical-progress";
export const KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_PATH = "/knowledge-lexical-progress";
export const KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_FLAG = "knowledgeLexicalProgress";
export const KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_TITLE = "Locus Full-Text Index";
export const KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT = "knowledge-lexical-rebuild-status";
export const LARGE_LEXICAL_REBUILD_DOC_THRESHOLD = 128;

export function isKnowledgeLexicalProgressWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_PATH
    || locationLike.search.includes(`${KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_FLAG}=1`);
}

export function buildKnowledgeLexicalProgressWindowQuery(): string {
  const params = new URLSearchParams({
    [KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_FLAG]: "1",
  });
  return params.toString();
}

export function buildKnowledgeLexicalProgressWindowUrl(): string {
  return buildSubWindowUrl(buildKnowledgeLexicalProgressWindowQuery());
}

export function shouldAutoOpenKnowledgeLexicalProgressWindow(
  status: LexicalRebuildStatus | null | undefined,
  threshold = LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
): boolean {
  if (!status?.running) return false;
  const totalDocs = status.totalDocs ?? 0;
  return totalDocs >= threshold;
}

export function getKnowledgeLexicalProgressRunKey(
  status: LexicalRebuildStatus | null | undefined,
): string {
  if (!status?.running) return "";
  return status.startedAt?.trim() || "active";
}

export async function openKnowledgeLexicalProgressWindow(
  _status?: LexicalRebuildStatus | null,
): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  await openSubWindow({
    kind: KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_LABEL,
    title: KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_TITLE,
    width: 560,
    height: 420,
    minWidth: 520,
    minHeight: 360,
    resizable: false,
    maximizable: false,
    minimizable: false,
    focusExisting: false,
  }, buildKnowledgeLexicalProgressWindowQuery());
}
