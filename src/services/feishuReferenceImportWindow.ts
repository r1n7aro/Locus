import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

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

export function buildFeishuReferenceImportWindowQuery(
  payload: FeishuReferenceImportWindowPayload = {},
): string {
  const params = new URLSearchParams({
    [FEISHU_REFERENCE_IMPORT_WINDOW_FLAG]: "1",
  });
  if (payload.targetPath?.trim()) {
    params.set("targetPath", payload.targetPath.trim());
  }
  return params.toString();
}

export function buildFeishuReferenceImportWindowUrl(
  payload: FeishuReferenceImportWindowPayload = {},
): string {
  return buildSubWindowUrl(buildFeishuReferenceImportWindowQuery(payload));
}

export async function openFeishuReferenceImportProgressWindow(
  payload: FeishuReferenceImportWindowPayload = {},
): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  const result = await openSubWindow({
    kind: FEISHU_REFERENCE_IMPORT_WINDOW_LABEL,
    title: FEISHU_REFERENCE_IMPORT_WINDOW_TITLE,
    width: 760,
    height: 760,
    minWidth: 700,
    minHeight: 680,
    resizable: true,
    maximizable: false,
    minimizable: false,
    closable: false,
  }, buildFeishuReferenceImportWindowQuery(payload));
  if (result.existing && payload.targetPath?.trim()) {
    await result.window?.emit(FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT, payload);
  }
}
