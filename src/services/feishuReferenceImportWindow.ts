import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

export const FEISHU_REFERENCE_IMPORT_WINDOW_LABEL = "feishu-reference-import-progress";
export const FEISHU_REFERENCE_IMPORT_WINDOW_PATH = "/feishu-reference-import";
export const FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT = "feishu-reference-import-progress:status";
export const FEISHU_REFERENCE_IMPORT_WINDOW_FLAG = "feishuReferenceImport";
export const FEISHU_REFERENCE_IMPORT_WINDOW_TITLE = "Locus Feishu Knowledge Base";

export interface FeishuReferenceImportWindowPayload {
  targetPath?: string | null;
}

export function isFeishuReferenceImportWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === FEISHU_REFERENCE_IMPORT_WINDOW_PATH
    || locationLike.search.includes(`${FEISHU_REFERENCE_IMPORT_WINDOW_FLAG}=1`);
}

export function getFeishuReferenceImportWindowPayload(
  search = window.location.search,
): FeishuReferenceImportWindowPayload {
  const params = new URLSearchParams(search);
  return {
    targetPath: params.get("targetPath")?.trim() || "",
  };
}

export function buildFeishuReferenceImportWindowUrl(
  payload: FeishuReferenceImportWindowPayload = {},
): string {
  const params = new URLSearchParams({
    [FEISHU_REFERENCE_IMPORT_WINDOW_FLAG]: "1",
  });
  if (payload.targetPath?.trim()) {
    params.set("targetPath", payload.targetPath.trim());
  }
  return `${FEISHU_REFERENCE_IMPORT_WINDOW_PATH}?${params.toString()}`;
}

export async function openFeishuReferenceImportProgressWindow(
  payload: FeishuReferenceImportWindowPayload = {},
): Promise<void> {
  const existingWindow = await WebviewWindow.getByLabel(FEISHU_REFERENCE_IMPORT_WINDOW_LABEL);
  if (existingWindow) {
    if (payload.targetPath?.trim()) {
      await existingWindow.emit(FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT, payload);
    }
    await existingWindow.setFocus();
    return;
  }

  await new Promise<void>((resolve, reject) => {
    const progressWindow = new WebviewWindow(FEISHU_REFERENCE_IMPORT_WINDOW_LABEL, {
      url: buildFeishuReferenceImportWindowUrl(payload),
      title: FEISHU_REFERENCE_IMPORT_WINDOW_TITLE,
      width: 760,
      height: 760,
      minWidth: 700,
      minHeight: 680,
      decorations: false,
      resizable: true,
      closable: false,
      minimizable: false,
      maximizable: false,
      center: true,
      shadow: true,
    });

    progressWindow.once("tauri://created", () => {
      resolve();
    });
    progressWindow.once("tauri://error", (event) => {
      reject(event);
    });
  });
}
