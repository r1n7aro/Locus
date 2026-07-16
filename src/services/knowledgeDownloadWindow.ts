import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

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

export function buildKnowledgeDownloadWindowQuery(modelId: string): string {
  const params = new URLSearchParams({
    [KNOWLEDGE_DOWNLOAD_WINDOW_FLAG]: "1",
    modelId,
  });
  return params.toString();
}

export function buildKnowledgeDownloadWindowUrl(modelId: string): string {
  return buildSubWindowUrl(buildKnowledgeDownloadWindowQuery(modelId));
}

export async function openKnowledgeDownloadProgressWindow(modelId: string): Promise<void> {
  const trimmedModelId = modelId.trim();
  if (!trimmedModelId) return;
  if (!hasTauriWindowRuntime()) return;

  const result = await openSubWindow({
    kind: KNOWLEDGE_DOWNLOAD_WINDOW_LABEL,
    title: KNOWLEDGE_DOWNLOAD_WINDOW_TITLE,
    width: 620,
    height: 560,
    minWidth: 600,
    minHeight: 520,
    resizable: false,
    maximizable: false,
    minimizable: false,
    closable: false,
    focusExisting: false,
  }, buildKnowledgeDownloadWindowQuery(trimmedModelId));
  if (result.existing) {
    await result.window?.emit(KNOWLEDGE_DOWNLOAD_WINDOW_MODEL_EVENT, {
      modelId: trimmedModelId,
    });
  }
}
