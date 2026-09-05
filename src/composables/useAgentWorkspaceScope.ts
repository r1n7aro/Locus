import { computed } from "vue";
import type { WorkspaceRef } from "../services/project";
import { useWorkbenchStore } from "../stores/workbench";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { useDisplaySettings } from "./useDisplaySettings";

/** The main Agent page follows the workspace tree in single-workspace mode. */
export function useAgentWorkspaceScope() {
  const workbenchStore = useWorkbenchStore();
  const workspaceContextStore = useWorkspaceContextStore();
  const { state: displaySettings } = useDisplaySettings();

  const checkout = computed(() => {
    if (displaySettings.workspaceDisplayMode !== "single") {
      return workspaceContextStore.focusedCheckout;
    }

    const checkoutId = workbenchStore.workspaceScope("main");
    if (checkoutId) return workspaceContextStore.checkoutsById[checkoutId] ?? null;

    // Bootstrap may restore the main pane before the tree adopts its scope.
    const paneId = workbenchStore.mainWindow?.focusedPaneId ?? "main";
    return workspaceContextStore.checkoutForPane("main", paneId);
  });
  const runtime = computed(() => checkout.value?.runtime ?? null);
  const workingDir = computed(() => checkout.value?.root ?? "");
  const workspaceRef = computed<WorkspaceRef | null>(() => {
    const current = runtime.value;
    return current ? {
      checkoutId: current.checkoutId,
      expectedGeneration: current.workspaceGeneration,
    } : null;
  });

  return { runtime, workingDir, workspaceRef };
}
