import type { LexicalRebuildStatus } from "../types";
import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { WorkspaceRef } from "./project";

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

export function buildKnowledgeLexicalProgressWindowQuery(workspaceRef?: WorkspaceRef): string {
  const params = new URLSearchParams({
    [KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_FLAG]: "1",
  });
  if (workspaceRef) {
    params.set("checkoutId", workspaceRef.checkoutId);
    params.set("workspaceGeneration", String(requireWindowGeneration(workspaceRef)));
  }
  return params.toString();
}

export function buildKnowledgeLexicalProgressWindowUrl(): string {
  return buildSubWindowUrl(buildKnowledgeLexicalProgressWindowQuery());
}

export function getKnowledgeLexicalProgressWindowWorkspaceRef(
  search = window.location.search,
): (WorkspaceRef & { expectedGeneration: number }) | null {
  const params = new URLSearchParams(search);
  const checkoutId = params.get("checkoutId")?.trim() ?? "";
  const generationText = params.get("workspaceGeneration") ?? "";
  if (!checkoutId || !/^\d+$/.test(generationText)) return null;
  const expectedGeneration = Number(generationText);
  if (!Number.isSafeInteger(expectedGeneration) || expectedGeneration < 0) return null;
  return { checkoutId, expectedGeneration };
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
  workspaceRef: WorkspaceRef,
  _status?: LexicalRebuildStatus | null,
): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  await openSubWindow({
    kind: `${KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_LABEL}-${workspaceWindowScope(workspaceRef)}`,
    title: KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_TITLE,
    width: 560,
    height: 420,
    minWidth: 520,
    minHeight: 360,
    resizable: false,
    maximizable: false,
    minimizable: false,
    focusExisting: false,
  }, buildKnowledgeLexicalProgressWindowQuery(workspaceRef));
}

function workspaceWindowScope(workspaceRef: WorkspaceRef): string {
  const checkoutToken = Array.from(new TextEncoder().encode(workspaceRef.checkoutId))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  return `${checkoutToken}-g${requireWindowGeneration(workspaceRef)}`;
}

function requireWindowGeneration(workspaceRef: WorkspaceRef): number {
  const generation = workspaceRef.expectedGeneration;
  if (!Number.isSafeInteger(generation) || Number(generation) < 0) {
    throw new Error("Lexical progress windows require an exact workspace generation.");
  }
  return Number(generation);
}
