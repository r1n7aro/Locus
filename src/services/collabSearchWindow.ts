import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { WorkspaceRef } from "./project";

export const COLLAB_SEARCH_WINDOW_LABEL = "collab-history-search";
export const COLLAB_SEARCH_WINDOW_PATH = "/collab-search";
export const COLLAB_SEARCH_WINDOW_FLAG = "collabSearch";
export const COLLAB_SEARCH_WINDOW_TITLE = "Locus Git Search";
export const COLLAB_SEARCH_SELECT_EVENT = "collab-search:select";

export interface CollabSearchSelectionPayload {
  kind: "commit" | "stash";
  hash: string;
  workspaceRef: WorkspaceRef;
}

export function isCollabSearchWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === COLLAB_SEARCH_WINDOW_PATH
    || locationLike.search.includes(`${COLLAB_SEARCH_WINDOW_FLAG}=1`);
}

export function getCollabSearchWindowWorkspaceRef(
  search = window.location.search,
): WorkspaceRef | null {
  const params = new URLSearchParams(search);
  const checkoutId = params.get("checkoutId")?.trim() || "";
  const generationRaw = params.get("workspaceGeneration");
  if (!checkoutId || !generationRaw || !/^\d+$/.test(generationRaw)) return null;
  const expectedGeneration = Number(generationRaw);
  if (!Number.isSafeInteger(expectedGeneration) || expectedGeneration < 0) return null;
  return { checkoutId, expectedGeneration };
}

export function buildCollabSearchWindowQuery(workspaceRef: WorkspaceRef): string {
  const expectedGeneration = requireWindowGeneration(workspaceRef);
  const params = new URLSearchParams({
    [COLLAB_SEARCH_WINDOW_FLAG]: "1",
    checkoutId: workspaceRef.checkoutId,
    workspaceGeneration: String(expectedGeneration),
  });
  return params.toString();
}

export function buildCollabSearchWindowUrl(workspaceRef: WorkspaceRef): string {
  return buildSubWindowUrl(buildCollabSearchWindowQuery(workspaceRef));
}

function searchWindowScopeKey(workspaceRef: WorkspaceRef): string {
  const checkoutToken = Array.from(new TextEncoder().encode(workspaceRef.checkoutId), byte => (
    byte.toString(16).padStart(2, "0")
  )).join("");
  return `${checkoutToken}-g${requireWindowGeneration(workspaceRef)}`;
}

function requireWindowGeneration(workspaceRef: WorkspaceRef): number {
  const generation = workspaceRef.expectedGeneration;
  if (!Number.isSafeInteger(generation) || Number(generation) < 0) {
    throw new Error("Collab search windows require an exact workspace generation.");
  }
  return Number(generation);
}

export async function openCollabSearchWindow(workspaceRef: WorkspaceRef): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  await openSubWindow({
    kind: `${COLLAB_SEARCH_WINDOW_LABEL}-${searchWindowScopeKey(workspaceRef)}`,
    title: COLLAB_SEARCH_WINDOW_TITLE,
    width: 960,
    height: 640,
    minWidth: 640,
    minHeight: 500,
    resizable: true,
    maximizable: false,
    minimizable: false,
  }, buildCollabSearchWindowQuery(workspaceRef));
}
