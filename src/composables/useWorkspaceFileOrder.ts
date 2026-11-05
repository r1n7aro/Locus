import { onBeforeUnmount, shallowRef, watch } from "vue";
import type { WorkspaceRef } from "../services/project";
import { normalizeAppError } from "../services/errors";
import { saveWorkspaceFileOrder, subscribeWorkspaceFileOrder, workspaceFileOrder,
  type WorkspaceFileOrder, type WorkspaceFilePlacement } from "../services/workspaceFileOrder";
import { subscribeExplorerFileActions } from "../services/workspaceExplorer";
import { applyWorkspaceFileOrder, minimalWorkspaceFileOrder } from "../components/explorer/workspaceFileOrder";

export function useWorkspaceFileOrder(props: { workingDir: string; workspaceRef: WorkspaceRef }, onError: (message: string) => void) {
  const order = shallowRef<WorkspaceFileOrder>({ revision: 0, placements: [] });
  const identity = () => JSON.stringify([props.workingDir, props.workspaceRef]);
  const rootKey = (value: string) => value.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  let disposed = false, request = 0;
  const releases: Array<() => void> = [];
  async function reload() {
    const token = ++request, key = identity();
    const value = await workspaceFileOrder(props.workspaceRef);
    if (!disposed && key === identity() && token === request) order.value = value;
    return value;
  }
  function refresh() { void reload().catch(error => { if (!disposed) onError(normalizeAppError(error).message); }); }
  watch(identity, () => { order.value = { revision: 0, placements: [] }; refresh(); }, { immediate: true });
  // The order is per-user state in the data directory, not a workspace file, so
  // saves and file actions are the only sources of change.
  for (const subscription of [
    subscribeWorkspaceFileOrder(event => { if (rootKey(event.root) === rootKey(props.workingDir)) refresh(); }),
    subscribeExplorerFileActions(event => { if (rootKey(event.root) === rootKey(props.workingDir)) refresh(); }),
  ]) void subscription.then(release => { if (disposed) release(); else releases.push(release); }).catch(() => {});
  onBeforeUnmount(() => { disposed = true; request++; releases.forEach(release => release()); });

  async function move(directory: string, item: string, anchor: string | null, side: "before" | "after", base: string[]) {
    const key = identity(), reference = { ...props.workspaceRef };
    for (let attempt = 0; attempt < 3; attempt++) {
      const current = await reload();
      if (disposed || key !== identity()) throw new Error("The workspace changed during the drag.");
      const existing = current.placements.filter(p => p.directory === directory);
      const desired = applyWorkspaceFileOrder(base, existing).filter(candidate => candidate !== item);
      if (!base.includes(item) || (anchor && !desired.includes(anchor))) throw new Error("The dragged item is no longer available.");
      const index = anchor ? desired.indexOf(anchor) + (side === "after" ? 1 : 0) : desired.length;
      desired.splice(index, 0, item);
      const known = new Set(base);
      const placements: WorkspaceFilePlacement[] = [
        // Hidden or unavailable anchors become active again when their rows return.
        ...existing.filter(p => !known.has(p.item) || (!known.has(p.anchor) && p.item !== item)),
        ...minimalWorkspaceFileOrder(base, desired, directory, new Set([item])),
      ];
      const seen = new Set<string>();
      const unique = [...placements].reverse().filter(p => !seen.has(p.item) && !!seen.add(p.item)).reverse();
      try {
        const saved = await saveWorkspaceFileOrder(reference, current.revision, directory, unique);
        if (!disposed && key === identity()) { request++; order.value = saved; }
        return;
      } catch (error) {
        if (normalizeAppError(error).code !== "workspace.file_order_revision_conflict" || attempt === 2) throw error;
      }
    }
  }
  return { order, move };
}
