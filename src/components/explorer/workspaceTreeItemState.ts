import type { ProjectExplorerItemRef, ProjectExplorerItemState, ProjectExplorerSnapshot } from "../../types/workbench";

export function sameWorkspaceTreeItem(left: ProjectExplorerItemRef, right: ProjectExplorerItemRef): boolean {
  return left.nodeId === right.nodeId && (left.relativePath || null) === (right.relativePath || null);
}

export function pinnedInsertionBefore(
  states: readonly ProjectExplorerItemState[],
  moving: readonly ProjectExplorerItemRef[],
  target: ProjectExplorerItemRef,
  after: boolean,
): ProjectExplorerItemRef | null {
  if (!after || moving.some((item) => sameWorkspaceTreeItem(item, target))) return target;
  const targetIndex = states.findIndex((state) => sameWorkspaceTreeItem(state, target));
  const next = states.slice(targetIndex + 1).find((state) => state.pinned
    && !moving.some((item) => sameWorkspaceTreeItem(item, state)));
  return next ? { nodeId: next.nodeId, relativePath: next.relativePath } : null;
}

export function workspaceTreeItemState(
  snapshot: ProjectExplorerSnapshot | undefined,
  nodeId: string,
  relativePath?: string | null,
): ProjectExplorerItemState | undefined {
  return snapshot?.itemStates?.find((state) => state.nodeId === nodeId
    && (state.relativePath ?? null) === (relativePath || null));
}

/** Partition a mounted directory without losing a pinned descendant when its
 * original ancestors are collapsed. Each pin owns its expanded subtree. */
export function mountedEntryInPinnedSubtree(
  relativePath: string,
  pinnedPaths: ReadonlySet<string>,
  pinnedRootPath?: string,
): boolean {
  const parts = relativePath.split("/");
  const closestPin = parts.map((_, index) => parts.slice(0, index + 1).join("/"))
    .reverse().find((path) => pinnedPaths.has(path));
  return closestPin === pinnedRootPath;
}
