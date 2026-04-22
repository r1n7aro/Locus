import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { UnityReferenceImportLocale, UnityReferenceImportStatus } from "../types";

export const UNITY_REFERENCE_IMPORT_WINDOW_LABEL = "unity-reference-import-progress";
export const UNITY_REFERENCE_IMPORT_WINDOW_PATH = "/unity-reference-import";
export const UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT = "unity-reference-import-progress:status";
export const UNITY_REFERENCE_IMPORT_WINDOW_FLAG = "unityReferenceImport";
export const UNITY_REFERENCE_IMPORT_WINDOW_TITLE = "Locus Unity Docs";

export interface UnityReferenceImportWindowPayload {
  targetPath?: string | null;
  running?: boolean;
  projectVersion?: string | null;
  docsVersion?: string | null;
  locale?: UnityReferenceImportLocale | null;
}

function normalizeUnityReferenceImportLocale(
  value: string | null | undefined,
): UnityReferenceImportLocale | undefined {
  if (value === "zh-CN") return "zh-CN";
  if (value === "en") return "en";
  return undefined;
}

function isUnityReferenceImportStatus(
  value: UnityReferenceImportStatus | UnityReferenceImportWindowPayload,
): value is UnityReferenceImportStatus {
  return "importedLocale" in value || "selectedLocale" in value;
}

export function isUnityReferenceImportWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === UNITY_REFERENCE_IMPORT_WINDOW_PATH
    || locationLike.search.includes(`${UNITY_REFERENCE_IMPORT_WINDOW_FLAG}=1`);
}

export function getUnityReferenceImportWindowPayload(
  search = window.location.search,
): UnityReferenceImportWindowPayload {
  const params = new URLSearchParams(search);
  return {
    targetPath: params.get("targetPath")?.trim() || "",
    running: params.get("running") === "1",
    projectVersion: params.get("projectVersion")?.trim() || "",
    docsVersion: params.get("docsVersion")?.trim() || "",
    locale: normalizeUnityReferenceImportLocale(params.get("locale")),
  };
}

export function buildUnityReferenceImportWindowUrl(
  payload: UnityReferenceImportWindowPayload = {},
): string {
  const params = new URLSearchParams({
    [UNITY_REFERENCE_IMPORT_WINDOW_FLAG]: "1",
  });
  if (payload.targetPath?.trim()) params.set("targetPath", payload.targetPath.trim());
  if (payload.running) params.set("running", "1");
  if (payload.projectVersion?.trim()) params.set("projectVersion", payload.projectVersion.trim());
  if (payload.docsVersion?.trim()) params.set("docsVersion", payload.docsVersion.trim());
  if (payload.locale) params.set("locale", payload.locale);
  return `${UNITY_REFERENCE_IMPORT_WINDOW_PATH}?${params.toString()}`;
}

function toWindowPayload(
  status: UnityReferenceImportStatus | UnityReferenceImportWindowPayload | null | undefined,
): UnityReferenceImportWindowPayload {
  if (!status) return {};
  const targetPath = isUnityReferenceImportStatus(status)
    ? status.managedPath?.trim().replace(/^reference\//, "")
    : status.targetPath;
  return {
    targetPath: targetPath?.trim() || "",
    running: !!status.running,
    projectVersion: status.projectVersion?.trim() || "",
    docsVersion: status.docsVersion?.trim() || "",
    locale: isUnityReferenceImportStatus(status)
      ? status.selectedLocale ?? status.importedLocale ?? undefined
      : status.locale,
  };
}

export async function openUnityReferenceImportProgressWindow(
  status?: UnityReferenceImportStatus | UnityReferenceImportWindowPayload | null,
): Promise<void> {
  const payload = toWindowPayload(status);
  const hasPayload = !!(
    payload.targetPath?.trim()
    || payload.running
    || payload.projectVersion?.trim()
    || payload.docsVersion?.trim()
    || payload.locale
  );
  const existingWindow = await WebviewWindow.getByLabel(UNITY_REFERENCE_IMPORT_WINDOW_LABEL);
  if (existingWindow) {
    if (hasPayload) {
      await existingWindow.emit(UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT, payload);
    }
    await existingWindow.setFocus();
    return;
  }

  await new Promise<void>((resolve, reject) => {
    const progressWindow = new WebviewWindow(UNITY_REFERENCE_IMPORT_WINDOW_LABEL, {
      url: buildUnityReferenceImportWindowUrl(payload),
      title: UNITY_REFERENCE_IMPORT_WINDOW_TITLE,
      width: 720,
      height: 560,
      minWidth: 680,
      minHeight: 500,
      decorations: false,
      resizable: false,
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
