import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { currentThemeBackgroundColor } from "../composables/useTheme";
import { hasTauriWindowRuntime } from "./tauriRuntime";

/** Shared open path for pooled sub-windows on the lightweight
 *  `window.html` entry. The Rust side (`commands/sub_window.rs`) keeps one
 *  pre-warmed hidden window; opening claims it (instant reveal) or falls
 *  back to a hidden direct creation that reveals after first paint. */

export const SUB_WINDOW_ENTRY_PATH = "/window.html";
export const SUB_WINDOW_ASSIGN_EVENT = "sub-window:assign";
export const SUB_WINDOW_POOL_FLAG = "subWindowPool";

export interface SubWindowDescriptor {
  /** Stable kind, also the window label for directly created windows. */
  kind: string;
  title: string;
  width: number;
  height: number;
  minWidth?: number;
  minHeight?: number;
  resizable?: boolean;
  maximizable?: boolean;
  minimizable?: boolean;
  closable?: boolean;
  /** Focus an already-open window of this kind (default true); quiet
   *  progress windows pass false to avoid stealing the foreground. */
  focusExisting?: boolean;
}

export interface SubWindowAssignPayload {
  kind: string;
  query: string;
}

export interface SubWindowOpenResult {
  existing: boolean;
  label: string;
  window: WebviewWindow | null;
}

export function buildSubWindowUrl(query: string): string {
  return `${SUB_WINDOW_ENTRY_PATH}?${query}`;
}

export function isSubWindowPoolLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  return locationLike.search.includes(`${SUB_WINDOW_POOL_FLAG}=1`);
}

export async function openSubWindow(
  descriptor: SubWindowDescriptor,
  query: string,
): Promise<SubWindowOpenResult> {
  const result = await invoke<{ label: string; existing: boolean; pooled: boolean }>(
    "sub_window_open",
    {
      request: {
        kind: descriptor.kind,
        query,
        title: descriptor.title,
        width: descriptor.width,
        height: descriptor.height,
        minWidth: descriptor.minWidth,
        minHeight: descriptor.minHeight,
        resizable: descriptor.resizable ?? true,
        maximizable: descriptor.maximizable ?? true,
        minimizable: descriptor.minimizable ?? false,
        closable: descriptor.closable ?? true,
        focusExisting: descriptor.focusExisting ?? true,
        backgroundColor: currentThemeBackgroundColor(),
      },
    },
  );
  const targetWindow = await WebviewWindow.getByLabel(result.label).catch(() => null);
  return { existing: result.existing, label: result.label, window: targetWindow };
}

/** Pre-warm the shared pool window; cheap no-op when one already waits. */
export async function prepareSubWindowPool(): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  await invoke("sub_window_pool_prepare", {
    backgroundColor: currentThemeBackgroundColor(),
  });
}

/** Called by the pool window itself once its assign listener is live. */
export async function markSubWindowPoolReady(label: string): Promise<void> {
  await invoke("sub_window_pool_ready", { label });
}

/** Latest open-request query recorded for a live window kind. Window
 *  components call this right after registering their payload listener to
 *  pick up a payload that was emitted (existing-window re-open) before the
 *  listener was live. */
export async function getSubWindowClaimedQuery(kind: string): Promise<string | null> {
  return (await invoke<string | null>("sub_window_claimed_query", { kind })) ?? null;
}
