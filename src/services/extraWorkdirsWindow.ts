import { emit, listen } from "@tauri-apps/api/event";
import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { WorkspaceRef } from "./project";

export const EXTRA_WORKDIRS_WINDOW_LABEL = "extra-workdirs-config";
export const EXTRA_WORKDIRS_WINDOW_PATH = "/extra-workdirs-config";
export const EXTRA_WORKDIRS_WINDOW_FLAG = "extraWorkdirsConfig";
export const EXTRA_WORKDIRS_WINDOW_TITLE = "Locus Additional Working Directories";
export const EXTRA_WORKDIRS_PAYLOAD_EVENT = "extra-workdirs-config:payload";
/**
 * Broadcast by the config window after a successful save so the main window
 * can refresh the workspace selector and re-check the current workspace —
 * the config window can edit any recent workspace, not just the active one.
 */
export const EXTRA_WORKDIRS_UPDATED_EVENT = "extra-workdirs:updated";

export interface ExtraWorkdirsWindowPayload {
  workspacePath: string;
  workspaceRef: WorkspaceRef;
}

export interface ExtraWorkdirsUpdatedEvent {
  workspacePath: string;
  workspaceRef: WorkspaceRef;
}

export function isExtraWorkdirsWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === EXTRA_WORKDIRS_WINDOW_PATH
    || locationLike.search.includes(`${EXTRA_WORKDIRS_WINDOW_FLAG}=1`);
}

export function getExtraWorkdirsWindowPayload(
  search = window.location.search,
): ExtraWorkdirsWindowPayload {
  const params = new URLSearchParams(search);
  const checkoutId = params.get("checkoutId")?.trim() || "";
  const generationRaw = params.get("workspaceGeneration") ?? "";
  const expectedGeneration = /^\d+$/.test(generationRaw)
    ? Number(generationRaw)
    : null;
  return {
    workspacePath: params.get("workspacePath")?.trim() || "",
    workspaceRef: {
      checkoutId,
      expectedGeneration: Number.isSafeInteger(expectedGeneration)
        ? expectedGeneration
        : undefined,
    },
  };
}

export function buildExtraWorkdirsWindowQuery(payload: ExtraWorkdirsWindowPayload): string {
  const params = new URLSearchParams({
    [EXTRA_WORKDIRS_WINDOW_FLAG]: "1",
    workspacePath: payload.workspacePath,
    checkoutId: payload.workspaceRef.checkoutId,
    workspaceGeneration: String(payload.workspaceRef.expectedGeneration ?? ""),
  });
  return params.toString();
}

export function buildExtraWorkdirsWindowUrl(payload: ExtraWorkdirsWindowPayload): string {
  return buildSubWindowUrl(buildExtraWorkdirsWindowQuery(payload));
}

export async function openExtraWorkdirsWindow(
  payload: ExtraWorkdirsWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;
  if (!payload.workspacePath.trim() || !payload.workspaceRef.checkoutId.trim()) return false;

  const result = await openSubWindow({
    kind: EXTRA_WORKDIRS_WINDOW_LABEL,
    title: EXTRA_WORKDIRS_WINDOW_TITLE,
    width: 640,
    height: 520,
    minWidth: 520,
    minHeight: 400,
    resizable: true,
    maximizable: false,
    minimizable: false,
  }, buildExtraWorkdirsWindowQuery(payload));
  if (result.existing) {
    await result.window?.emit(EXTRA_WORKDIRS_PAYLOAD_EVENT, payload);
  }
  return true;
}

export function broadcastExtraWorkdirsUpdated(
  workspacePath: string,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return emit(EXTRA_WORKDIRS_UPDATED_EVENT, {
    workspacePath,
    workspaceRef,
  } satisfies ExtraWorkdirsUpdatedEvent);
}

export function listenExtraWorkdirsUpdated(
  handler: (event: ExtraWorkdirsUpdatedEvent) => void,
): Promise<() => void> {
  if (!hasTauriWindowRuntime()) return Promise.resolve(() => {});
  return listen<ExtraWorkdirsUpdatedEvent>(EXTRA_WORKDIRS_UPDATED_EVENT, (event) => {
    if (
      event.payload
      && typeof event.payload.workspacePath === "string"
      && typeof event.payload.workspaceRef?.checkoutId === "string"
    ) {
      handler(event.payload);
    }
  });
}
