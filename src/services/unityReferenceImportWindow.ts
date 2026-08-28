import type { UnityReferenceImportLocale, UnityReferenceImportStatus } from "../types";
import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { WorkspaceRef } from "./project";

export const UNITY_REFERENCE_IMPORT_WINDOW_LABEL = "unity-reference-import-progress";
export const UNITY_REFERENCE_IMPORT_WINDOW_PATH = "/unity-reference-import";
export const UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT = "unity-reference-import-progress:status";
export const UNITY_REFERENCE_IMPORT_WINDOW_FLAG = "unityReferenceImport";
export const UNITY_REFERENCE_IMPORT_WINDOW_TITLE = "Locus Unity Docs";

export interface UnityReferenceImportWindowPayload {
  workspaceRef?: WorkspaceRef | null;
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
    workspaceRef: workspaceRefFromParams(params),
    targetPath: params.get("targetPath")?.trim() || "",
    running: params.get("running") === "1",
    projectVersion: params.get("projectVersion")?.trim() || "",
    docsVersion: params.get("docsVersion")?.trim() || "",
    locale: normalizeUnityReferenceImportLocale(params.get("locale")),
  };
}

export function buildUnityReferenceImportWindowQuery(
  payload: UnityReferenceImportWindowPayload = {},
): string {
  const params = new URLSearchParams({
    [UNITY_REFERENCE_IMPORT_WINDOW_FLAG]: "1",
  });
  appendWorkspaceRef(params, payload.workspaceRef);
  if (payload.targetPath?.trim()) params.set("targetPath", payload.targetPath.trim());
  if (payload.running) params.set("running", "1");
  if (payload.projectVersion?.trim()) params.set("projectVersion", payload.projectVersion.trim());
  if (payload.docsVersion?.trim()) params.set("docsVersion", payload.docsVersion.trim());
  if (payload.locale) params.set("locale", payload.locale);
  return params.toString();
}

export function buildUnityReferenceImportWindowUrl(
  payload: UnityReferenceImportWindowPayload = {},
): string {
  return buildSubWindowUrl(buildUnityReferenceImportWindowQuery(payload));
}

function toWindowPayload(
  workspaceRef: WorkspaceRef,
  status: UnityReferenceImportStatus | UnityReferenceImportWindowPayload | null | undefined,
): UnityReferenceImportWindowPayload {
  if (!status) return { workspaceRef };
  const targetPath = isUnityReferenceImportStatus(status)
    ? status.managedPath?.trim().replace(/^reference\//, "")
    : status.targetPath;
  return {
    workspaceRef,
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
  workspaceRef: WorkspaceRef,
  status?: UnityReferenceImportStatus | UnityReferenceImportWindowPayload | null,
): Promise<void> {
  const payload = toWindowPayload(workspaceRef, status);
  const hasPayload = !!(
    payload.targetPath?.trim()
    || payload.running
    || payload.projectVersion?.trim()
    || payload.docsVersion?.trim()
    || payload.locale
  );
  if (!hasTauriWindowRuntime()) return;
  const result = await openSubWindow({
    kind: `${UNITY_REFERENCE_IMPORT_WINDOW_LABEL}-${safeWindowScope(workspaceRef.checkoutId)}`,
    title: UNITY_REFERENCE_IMPORT_WINDOW_TITLE,
    width: 720,
    height: 560,
    minWidth: 680,
    minHeight: 500,
    resizable: false,
    maximizable: false,
    minimizable: false,
    closable: false,
  }, buildUnityReferenceImportWindowQuery(payload));
  if (result.existing && hasPayload) {
    await result.window?.emit(UNITY_REFERENCE_IMPORT_WINDOW_STATUS_EVENT, payload);
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
