import { currentThemeBackgroundColor } from "../composables/useTheme";
import { ipcInvoke } from "./ipc";
import { openWorkspace, materializationEpochFromParams, type WorkspaceRef } from "./project";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const WORKSPACE_WINDOW_FLAG = "workspaceWindow";
export const WORKSPACE_WINDOW_LABEL_PREFIX = "workspace-app-";
/** Setting: workspace windows other than the primary one also show the page tab bar. */
export const WORKSPACE_WINDOW_TITLE_BAR_SETTING = "appearance.chrome.workspaceWindowTitleBar";

export function isWorkspaceWindowLabel(label: string): boolean {
  return label.startsWith(WORKSPACE_WINDOW_LABEL_PREFIX);
}

export function workspaceWindowRef(search: string): WorkspaceRef {
  const params = new URLSearchParams(search);
  const checkoutId = params.get("checkoutId")?.trim();
  const generation = params.get("workspaceGeneration");
  if (params.get(WORKSPACE_WINDOW_FLAG) !== "1" || !checkoutId
    || !generation || !/^\d+$/.test(generation)
    || !Number.isSafeInteger(Number(generation))) {
    throw new Error("Invalid workspace window scope.");
  }
  return {
    checkoutId,
    expectedGeneration: Number(generation),
    expectedMaterializationEpoch: materializationEpochFromParams(params),
  };
}

export function workspaceWindowQuery(workspaceRef: WorkspaceRef): string {
  const params = new URLSearchParams({
    [WORKSPACE_WINDOW_FLAG]: "1",
    checkoutId: workspaceRef.checkoutId,
    workspaceGeneration: String(workspaceRef.expectedGeneration),
  });
  if (workspaceRef.expectedMaterializationEpoch != null) {
    params.set("materializationEpoch", String(workspaceRef.expectedMaterializationEpoch));
  }
  // Use the same validation for generated and received URLs.
  workspaceWindowRef(params.toString());
  return params.toString();
}

/** Opening a runtime does not change the source window's focused pane. */
export async function openWorkspaceWindow(path: string): Promise<string> {
  if (!path.trim()) throw new Error("A workspace path is required.");
  if (!hasTauriWindowRuntime()) throw new Error("Workspace windows require the desktop app.");
  const runtime = await openWorkspace(path);
  return ipcInvoke<string>("open_workspace_window", {
    workspaceRef: {
      checkoutId: runtime.checkoutId,
      expectedGeneration: runtime.workspaceGeneration,
      expectedMaterializationEpoch: runtime.materializationEpoch,
    },
    backgroundColor: currentThemeBackgroundColor(),
  });
}
