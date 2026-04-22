import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";

export const KNOWLEDGE_DOWNLOAD_WINDOW_LABEL = "knowledge-download-progress";
export const KNOWLEDGE_DOWNLOAD_WINDOW_PATH = "/knowledge-download";
export const KNOWLEDGE_DOWNLOAD_WINDOW_MODEL_EVENT = "knowledge-download-progress:model";
export const KNOWLEDGE_DOWNLOAD_WINDOW_FLAG = "knowledgeDownload";
export const KNOWLEDGE_DOWNLOAD_WINDOW_TITLE = "Locus Downloading..";

export function isKnowledgeDownloadWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === KNOWLEDGE_DOWNLOAD_WINDOW_PATH
    || locationLike.search.includes(`${KNOWLEDGE_DOWNLOAD_WINDOW_FLAG}=1`);
}

export function getKnowledgeDownloadWindowModelId(search = window.location.search): string {
  const params = new URLSearchParams(search);
  return params.get("modelId")?.trim() || "";
}

export function buildKnowledgeDownloadWindowUrl(modelId: string): string {
  const params = new URLSearchParams({
    [KNOWLEDGE_DOWNLOAD_WINDOW_FLAG]: "1",
    modelId,
  });
  return `${KNOWLEDGE_DOWNLOAD_WINDOW_PATH}?${params.toString()}`;
}

export async function openKnowledgeDownloadProgressWindow(modelId: string): Promise<void> {
  const trimmedModelId = modelId.trim();
  if (!trimmedModelId) return;

  const existingWindow = await WebviewWindow.getByLabel(KNOWLEDGE_DOWNLOAD_WINDOW_LABEL);
  if (existingWindow) {
    await existingWindow.emit(KNOWLEDGE_DOWNLOAD_WINDOW_MODEL_EVENT, {
      modelId: trimmedModelId,
    });
    return;
  }

  await new Promise<void>((resolve, reject) => {
    const downloadWindow = new WebviewWindow(KNOWLEDGE_DOWNLOAD_WINDOW_LABEL, {
      url: buildKnowledgeDownloadWindowUrl(trimmedModelId),
      title: KNOWLEDGE_DOWNLOAD_WINDOW_TITLE,
      width: 620,
      height: 560,
      minWidth: 600,
      minHeight: 520,
      decorations: false,
      resizable: false,
      closable: false,
      minimizable: false,
      maximizable: false,
      parent: getCurrentWebviewWindow(),
      center: true,
    });

    downloadWindow.once("tauri://created", () => {
      resolve();
    });
    downloadWindow.once("tauri://error", (event) => {
      reject(event);
    });
  });
}
