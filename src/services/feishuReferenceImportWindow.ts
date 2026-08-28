import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { WorkspaceRef } from "./project";

export const FEISHU_REFERENCE_IMPORT_WINDOW_LABEL = "feishu-reference-import-progress";
export const FEISHU_REFERENCE_IMPORT_WINDOW_PATH = "/feishu-reference-import";
export const FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT = "feishu-reference-import-progress:status";
export const FEISHU_REFERENCE_IMPORT_WINDOW_FLAG = "feishuReferenceImport";
export const FEISHU_REFERENCE_IMPORT_WINDOW_TITLE = "Locus Feishu Knowledge Base";

export interface FeishuReferenceImportWindowPayload {
  workspaceRef?: WorkspaceRef | null;
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
    workspaceRef: workspaceRefFromParams(params),
    targetPath: params.get("targetPath")?.trim() || "",
  };
}

export function buildFeishuReferenceImportWindowQuery(
  payload: FeishuReferenceImportWindowPayload = {},
): string {
  const params = new URLSearchParams({
    [FEISHU_REFERENCE_IMPORT_WINDOW_FLAG]: "1",
  });
  appendWorkspaceRef(params, payload.workspaceRef);
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
  payload: FeishuReferenceImportWindowPayload & { workspaceRef: WorkspaceRef },
): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  const result = await openSubWindow({
    kind: `${FEISHU_REFERENCE_IMPORT_WINDOW_LABEL}-${safeWindowScope(payload.workspaceRef.checkoutId)}`,
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

function appendWorkspaceRef(params: URLSearchParams, workspaceRef?: WorkspaceRef | null) {
  if (!workspaceRef?.checkoutId) return;
  params.set("checkoutId", workspaceRef.checkoutId);
  if (workspaceRef.expectedGeneration != null) {
    params.set("workspaceGeneration", String(workspaceRef.expectedGeneration));
  }
}

function workspaceRefFromParams(params: URLSearchParams): WorkspaceRef | null {
  const checkoutId = params.get("checkoutId")?.trim() ?? "";
  if (!checkoutId) return null;
  const generation = Number(params.get("workspaceGeneration"));
  return {
    checkoutId,
    expectedGeneration: Number.isSafeInteger(generation) && generation > 0
      ? generation
      : undefined,
  };
}

function safeWindowScope(value: string): string {
  return Array.from(new TextEncoder().encode(value))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}
