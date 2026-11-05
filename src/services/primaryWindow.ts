import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { ipcInvoke } from "./ipc";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const PRIMARY_WINDOW_CHANGED_EVENT = "locus-primary-window-changed";

/**
 * Process-wide renderer duties (hosting Unity embedded workbenches, handling
 * external script opens, the startup update check) run in exactly one
 * application window: the oldest one still open. No window is special; when
 * the primary window closes the next oldest takes over.
 *
 * `onChange` receives the current answer first, then again each time the
 * primary window changes. Outside the desktop runtime the window is primary.
 */
export async function watchPrimaryWindow(
  windowLabel: string,
  onChange: (isPrimary: boolean) => void,
): Promise<UnlistenFn> {
  if (!hasTauriWindowRuntime()) {
    onChange(true);
    return () => {};
  }
  let announced = false;
  const release = await listen<string | null>(PRIMARY_WINDOW_CHANGED_EVENT, (event) => {
    announced = true;
    onChange(event.payload === windowLabel);
  });
  try {
    const primary = await ipcInvoke<string | null>("get_primary_application_window");
    // A change announced while the query was in flight is newer than its answer.
    if (!announced) onChange(primary === windowLabel);
  } catch (error) {
    console.warn("[window] failed to query the primary window", error);
  }
  return release;
}
