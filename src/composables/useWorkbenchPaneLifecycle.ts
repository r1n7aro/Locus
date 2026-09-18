import { watch } from "vue";
import { useWorkspaceContextStore } from "../stores/workspaceContext";

export function useWorkbenchPaneLifecycle(windowId: string, paneIds: () => readonly string[]) {
  const workspace = useWorkspaceContextStore();
  return watch(
    () => workspace.initialized ? [...paneIds()].sort().join("\u0000") : null,
    (visible) => {
      if (visible === null) return;
      // Saved layouts belong in local storage. Only the rendered layout owns
      // pane leases; hidden checkout layouts must be able to become idle.
      void workspace.reconcileWindowPanes(windowId, visible ? visible.split("\u0000") : [])
        .catch((error) => {
          console.warn("[DevelopmentWorkbench] pane lifecycle reconciliation failed", error);
        });
    },
    { immediate: true, flush: "sync" },
  );
}
