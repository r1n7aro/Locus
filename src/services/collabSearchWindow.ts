import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const COLLAB_SEARCH_WINDOW_LABEL = "collab-history-search";
export const COLLAB_SEARCH_WINDOW_PATH = "/collab-search";
export const COLLAB_SEARCH_WINDOW_FLAG = "collabSearch";
export const COLLAB_SEARCH_WINDOW_TITLE = "Locus Git Search";
export const COLLAB_SEARCH_SELECT_EVENT = "collab-search:select";

export interface CollabSearchSelectionPayload {
  kind: "commit" | "stash";
  hash: string;
}

export function isCollabSearchWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === COLLAB_SEARCH_WINDOW_PATH
    || locationLike.search.includes(`${COLLAB_SEARCH_WINDOW_FLAG}=1`);
}

export function buildCollabSearchWindowQuery(): string {
  const params = new URLSearchParams({
    [COLLAB_SEARCH_WINDOW_FLAG]: "1",
  });
  return params.toString();
}

export function buildCollabSearchWindowUrl(): string {
  return buildSubWindowUrl(buildCollabSearchWindowQuery());
}

export async function openCollabSearchWindow(): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  await openSubWindow({
    kind: COLLAB_SEARCH_WINDOW_LABEL,
    title: COLLAB_SEARCH_WINDOW_TITLE,
    width: 960,
    height: 640,
    minWidth: 640,
    minHeight: 500,
    resizable: true,
    maximizable: false,
    minimizable: false,
  }, buildCollabSearchWindowQuery());
}
