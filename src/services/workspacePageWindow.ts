import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const WORKSPACE_PAGE_WINDOW_FLAG = "workspacePageWindow";
export const WORKSPACE_PAGE_RESET_ONBOARDING_EVENT = "workspace-page:reset-onboarding";

export const CHECKOUT_WORKSPACE_PAGE_IDS = [
  "chat",
  "knowledge",
  "collab",
  "asset",
  "views",
  "agent",
] as const;

export const APP_WORKSPACE_PAGE_IDS = [
  "plugins",
  "settings",
] as const;

export const WORKSPACE_PAGE_IDS = [
  ...CHECKOUT_WORKSPACE_PAGE_IDS,
  ...APP_WORKSPACE_PAGE_IDS,
] as const;

export type CheckoutWorkspacePageId = typeof CHECKOUT_WORKSPACE_PAGE_IDS[number];
export type AppWorkspacePageId = typeof APP_WORKSPACE_PAGE_IDS[number];
export type WorkspacePageId = typeof WORKSPACE_PAGE_IDS[number];

export interface CheckoutWorkspacePageWindowPayload {
  scope: "checkout";
  page: CheckoutWorkspacePageId;
  checkoutId: string;
  workspaceGeneration: number;
  title: string;
}

export interface AppWorkspacePageWindowPayload {
  scope: "app";
  page: AppWorkspacePageId;
  title: string;
}

export type WorkspacePageWindowPayload =
  | CheckoutWorkspacePageWindowPayload
  | AppWorkspacePageWindowPayload;

export function isCheckoutWorkspacePageId(
  value: string | null | undefined,
): value is CheckoutWorkspacePageId {
  return CHECKOUT_WORKSPACE_PAGE_IDS.includes(value as CheckoutWorkspacePageId);
}

export function isAppWorkspacePageId(
  value: string | null | undefined,
): value is AppWorkspacePageId {
  return APP_WORKSPACE_PAGE_IDS.includes(value as AppWorkspacePageId);
}

export function isWorkspacePageId(value: string | null | undefined): value is WorkspacePageId {
  return isCheckoutWorkspacePageId(value) || isAppWorkspacePageId(value);
}

function isWorkspaceGeneration(value: unknown): value is number {
  return typeof value === "number"
    && Number.isSafeInteger(value)
    && value >= 0;
}

function normalizedTitle(title: string, page: WorkspacePageId): string {
  return title.trim() || page;
}

function normalizeCanonicalPayload(
  payload: WorkspacePageWindowPayload,
): WorkspacePageWindowPayload | null {
  if (payload.scope === "app") {
    if (!isAppWorkspacePageId(payload.page)) return null;
    return {
      scope: "app",
      page: payload.page,
      title: normalizedTitle(payload.title, payload.page),
    };
  }

  const checkoutId = payload.checkoutId.trim();
  if (
    !isCheckoutWorkspacePageId(payload.page)
    || !checkoutId
    || !isWorkspaceGeneration(payload.workspaceGeneration)
  ) {
    return null;
  }
  return {
    scope: "checkout",
    page: payload.page,
    checkoutId,
    workspaceGeneration: payload.workspaceGeneration,
    title: normalizedTitle(payload.title, payload.page),
  };
}

function parseWorkspaceGeneration(value: string | null): number | null {
  if (value == null || !/^\d+$/.test(value)) return null;
  const generation = Number(value);
  return isWorkspaceGeneration(generation) ? generation : null;
}

function safeCheckoutWindowKey(checkoutId: string): string {
  // Hex-encoded UTF-8 is injective and stays inside the native window-label
  // allowlist. A readable slug can collide after punctuation normalization.
  return Array.from(new TextEncoder().encode(checkoutId), (byte) => (
    byte.toString(16).padStart(2, "0")
  )).join("");
}

export function workspacePageWindowKind(payload: WorkspacePageWindowPayload): string {
  const normalized = normalizeCanonicalPayload(payload);
  if (!normalized) {
    throw new Error("Invalid workspace page window payload.");
  }
  if (normalized.scope === "app") {
    return `workspace-page-${normalized.page}`;
  }
  return `workspace-page-${normalized.page}-${safeCheckoutWindowKey(normalized.checkoutId)}-g${normalized.workspaceGeneration.toString(16)}`;
}

export function isWorkspacePageWindowLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  const params = new URLSearchParams(locationLike.search);
  return params.get(WORKSPACE_PAGE_WINDOW_FLAG) === "1"
    && getWorkspacePageWindowPayload(locationLike.search) !== null;
}

export function getWorkspacePageWindowPayload(
  search = window.location.search,
): WorkspacePageWindowPayload | null {
  const params = new URLSearchParams(search);
  const page = params.get("page");
  if (!isWorkspacePageId(page)) return null;

  const scope = params.get("scope");
  const title = params.get("title")?.trim() || page;
  const hasCheckoutId = params.has("checkoutId");
  const hasWorkspaceGeneration = params.has("workspaceGeneration");
  const checkoutId = params.get("checkoutId")?.trim() || "";
  const workspaceGeneration = parseWorkspaceGeneration(params.get("workspaceGeneration"));

  if (scope === "app") {
    if (!isAppWorkspacePageId(page) || hasCheckoutId || hasWorkspaceGeneration) return null;
    return { scope: "app", page, title };
  }

  if (scope === "checkout") {
    if (!isCheckoutWorkspacePageId(page) || !checkoutId || workspaceGeneration === null) {
      return null;
    }
    return { scope: "checkout", page, checkoutId, workspaceGeneration, title };
  }

  if (scope !== null) return null;

  // Legacy URLs had no scope. Process-level pages have an unambiguous owner,
  // while checkout pages cannot safely recover an implicit current workspace.
  if (isAppWorkspacePageId(page) && !hasCheckoutId && !hasWorkspaceGeneration) {
    return { scope: "app", page, title };
  }
  return null;
}

export function buildWorkspacePageWindowQuery(payload: WorkspacePageWindowPayload): string {
  const normalized = normalizeCanonicalPayload(payload);
  if (!normalized) {
    throw new Error("Invalid workspace page window payload.");
  }

  const params = new URLSearchParams({
    [WORKSPACE_PAGE_WINDOW_FLAG]: "1",
    scope: normalized.scope,
    page: normalized.page,
    title: normalized.title,
  });
  if (normalized.scope === "checkout") {
    params.set("checkoutId", normalized.checkoutId);
    params.set("workspaceGeneration", String(normalized.workspaceGeneration));
  }
  return params.toString();
}

export function buildWorkspacePageWindowUrl(payload: WorkspacePageWindowPayload): string {
  return buildSubWindowUrl(buildWorkspacePageWindowQuery(payload));
}

export async function openWorkspacePageWindow(
  payload: WorkspacePageWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const normalized = normalizeCanonicalPayload(payload);
  if (!normalized) throw new Error("Invalid workspace page window payload.");
  await openSubWindow({
    kind: workspacePageWindowKind(normalized),
    title: `Locus - ${normalized.title}`,
    width: 1280,
    height: 820,
    minWidth: 760,
    minHeight: 520,
    resizable: true,
    maximizable: true,
    minimizable: true,
    closable: true,
  }, buildWorkspacePageWindowQuery(normalized));
  return true;
}
