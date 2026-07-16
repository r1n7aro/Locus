import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export type ReferenceExternalImportSource = "feishu" | "unity" | "local";

export const REFERENCE_EXTERNAL_IMPORT_WINDOW_LABEL = "reference-external-import";
export const REFERENCE_EXTERNAL_IMPORT_WINDOW_PATH = "/reference-external-import";
export const REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT = "reference-external-import:payload";
export const REFERENCE_EXTERNAL_IMPORT_WINDOW_FLAG = "referenceExternalImport";
export const REFERENCE_EXTERNAL_IMPORT_WINDOW_TITLE = "Locus External Import";

export interface ReferenceExternalImportWindowPayload {
  parentDir?: string | null;
  fixedTargetPath?: string | null;
  initialSource?: ReferenceExternalImportSource | null;
}

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

export function isReferenceExternalImportWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === REFERENCE_EXTERNAL_IMPORT_WINDOW_PATH
    || locationLike.search.includes(`${REFERENCE_EXTERNAL_IMPORT_WINDOW_FLAG}=1`);
}

export function getReferenceExternalImportWindowPayload(
  search = window.location.search,
): ReferenceExternalImportWindowPayload {
  const params = new URLSearchParams(search);
  const initialSource = params.get("initialSource");
  return {
    parentDir: trimOrEmpty(params.get("parentDir")),
    fixedTargetPath: trimOrEmpty(params.get("fixedTargetPath")),
    initialSource: initialSource === "unity" || initialSource === "feishu" || initialSource === "local"
      ? initialSource
      : null,
  };
}

export function buildReferenceExternalImportWindowQuery(
  payload: ReferenceExternalImportWindowPayload = {},
): string {
  const params = new URLSearchParams({
    [REFERENCE_EXTERNAL_IMPORT_WINDOW_FLAG]: "1",
  });
  if (trimOrEmpty(payload.parentDir)) {
    params.set("parentDir", trimOrEmpty(payload.parentDir));
  }
  if (trimOrEmpty(payload.fixedTargetPath)) {
    params.set("fixedTargetPath", trimOrEmpty(payload.fixedTargetPath));
  }
  if (
    payload.initialSource === "feishu"
    || payload.initialSource === "unity"
    || payload.initialSource === "local"
  ) {
    params.set("initialSource", payload.initialSource);
  }
  return params.toString();
}

export function buildReferenceExternalImportWindowUrl(
  payload: ReferenceExternalImportWindowPayload = {},
): string {
  return buildSubWindowUrl(buildReferenceExternalImportWindowQuery(payload));
}

export async function openReferenceExternalImportWindow(
  payload: ReferenceExternalImportWindowPayload = {},
): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  const result = await openSubWindow({
    kind: REFERENCE_EXTERNAL_IMPORT_WINDOW_LABEL,
    title: REFERENCE_EXTERNAL_IMPORT_WINDOW_TITLE,
    width: 1180,
    height: 900,
    minWidth: 920,
    minHeight: 700,
    resizable: true,
    maximizable: false,
    minimizable: false,
    closable: false,
  }, buildReferenceExternalImportWindowQuery(payload));
  if (result.existing) {
    await result.window?.emit(REFERENCE_EXTERNAL_IMPORT_WINDOW_EVENT, payload);
  }
}
