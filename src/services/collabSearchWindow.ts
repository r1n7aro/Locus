import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

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

export function buildCollabSearchWindowUrl(): string {
  const params = new URLSearchParams({
    [COLLAB_SEARCH_WINDOW_FLAG]: "1",
  });
  return `${COLLAB_SEARCH_WINDOW_PATH}?${params.toString()}`;
}

export async function openCollabSearchWindow(): Promise<void> {
  const existingWindow = await WebviewWindow.getByLabel(COLLAB_SEARCH_WINDOW_LABEL);
  if (existingWindow) {
    await existingWindow.setFocus();
    return;
  }

  await new Promise<void>((resolve, reject) => {
    const searchWindow = new WebviewWindow(COLLAB_SEARCH_WINDOW_LABEL, {
      url: buildCollabSearchWindowUrl(),
      title: COLLAB_SEARCH_WINDOW_TITLE,
      width: 960,
      height: 640,
      minWidth: 640,
      minHeight: 500,
      decorations: false,
      resizable: true,
      closable: true,
      minimizable: false,
      maximizable: false,
      parent: "main",
      center: true,
      shadow: true,
    });

    searchWindow.once("tauri://created", () => {
      resolve();
    });
    searchWindow.once("tauri://error", (event) => {
      reject(event);
    });
  });
}
