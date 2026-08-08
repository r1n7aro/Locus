import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const WORKSPACE_PAGE_WINDOW_FLAG = "workspacePageWindow";
export const WORKSPACE_PAGE_RESET_ONBOARDING_EVENT = "workspace-page:reset-onboarding";

export const WORKSPACE_PAGE_IDS = [
  "knowledge",
  "collab",
  "asset",
  "views",
  "plugins",
  "agent",
  "settings",
] as const;

export type WorkspacePageId = typeof WORKSPACE_PAGE_IDS[number];

export interface WorkspacePageWindowPayload {
  page: WorkspacePageId;
  title: string;
}

export function isWorkspacePageId(value: string | null | undefined): value is WorkspacePageId {
  return WORKSPACE_PAGE_IDS.includes(value as WorkspacePageId);
}

export function workspacePageWindowKind(page: WorkspacePageId): string {
  return `workspace-page-${page}`;
}

export function isWorkspacePageWindowLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  const params = new URLSearchParams(locationLike.search);
  return params.get(WORKSPACE_PAGE_WINDOW_FLAG) === "1"
    && isWorkspacePageId(params.get("page"));
}

export function getWorkspacePageWindowPayload(
  search = window.location.search,
): WorkspacePageWindowPayload | null {
  const params = new URLSearchParams(search);
  const page = params.get("page");
  if (!isWorkspacePageId(page)) return null;
  return {
    page,
    title: params.get("title")?.trim() || page,
  };
}

export function buildWorkspacePageWindowQuery(payload: WorkspacePageWindowPayload): string {
  return new URLSearchParams({
    [WORKSPACE_PAGE_WINDOW_FLAG]: "1",
    page: payload.page,
    title: payload.title.trim() || payload.page,
  }).toString();
}

export function buildWorkspacePageWindowUrl(payload: WorkspacePageWindowPayload): string {
  return buildSubWindowUrl(buildWorkspacePageWindowQuery(payload));
}

export async function openWorkspacePageWindow(
  payload: WorkspacePageWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const title = payload.title.trim() || payload.page;
  await openSubWindow({
    kind: workspacePageWindowKind(payload.page),
    title: `Locus - ${title}`,
    width: 1280,
    height: 820,
    minWidth: 760,
    minHeight: 520,
    resizable: true,
    maximizable: true,
    minimizable: true,
    closable: true,
  }, buildWorkspacePageWindowQuery({ ...payload, title }));
  return true;
}
