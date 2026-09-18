import type { ProjectExplorerItemRef, ProjectExplorerNode, ProjectExplorerOperation, ProjectExplorerSnapshot } from "../../types/workbench";
import { sameWorkspaceTreeItem } from "./workspaceTreeItemState";

type MoveOperation = Extract<ProjectExplorerOperation, { kind: "moveNode" }>;

export function workspaceTreePinOperations(
  items: ProjectExplorerItemRef[],
  before?: ProjectExplorerItemRef | null,
): ProjectExplorerOperation[] {
  return [
    ...items.map((item) => ({ kind: "setItemState" as const, ...item, pinned: true })),
    ...(before !== undefined ? [{ kind: "movePinnedItems" as const, items, before }] : []),
  ];
}

export function previewWorkspaceTreePin(
  snapshot: ProjectExplorerSnapshot,
  items: ProjectExplorerItemRef[],
  before?: ProjectExplorerItemRef | null,
): ProjectExplorerSnapshot {
  const states = (snapshot.itemStates ?? []).map((state) => ({ ...state }));
  for (const item of items) {
    const state = states.find((candidate) => sameWorkspaceTreeItem(candidate, item));
    if (state) state.pinned = true;
    else states.push({ ...item, pinned: true, highlighted: false });
  }
  // The backend preserves the saved order within a multi-item move and treats
  // dropping onto one of the moving items as a no-op.
  if (before === undefined || (before && items.some((item) => sameWorkspaceTreeItem(item, before)))) {
    return { ...snapshot, itemStates: states };
  }
  const moving = states.filter((state) => items.some((item) => sameWorkspaceTreeItem(state, item)));
  const remaining = states.filter((state) => !items.some((item) => sameWorkspaceTreeItem(state, item)));
  const index = before ? remaining.findIndex((state) => sameWorkspaceTreeItem(state, before)) : -1;
  remaining.splice(index < 0 ? remaining.length : index, 0, ...moving);
  return { ...snapshot, itemStates: remaining };
}

export function workspaceTreeMoveOperation(
  source: ProjectExplorerNode,
  intent: { parentNodeId: string | null; position: number },
): MoveOperation {
  const position = (source.parentNodeId ?? null) === intent.parentNodeId
    && source.position < intent.position
    ? Math.max(0, intent.position - 1)
    : intent.position;
  return { kind: "moveNode", nodeId: source.nodeId, parentNodeId: intent.parentNodeId, position };
}

/** Keep a multi-selection together even when earlier moves shift sibling indices. */
export function workspaceTreeMoveOperations(
  snapshot: ProjectExplorerSnapshot,
  sources: ProjectExplorerNode[],
  intent: { parentNodeId: string | null; position: number },
): MoveOperation[] {
  const moving = new Set(sources.map((node) => node.nodeId));
  const siblings = (state: ProjectExplorerSnapshot) => state.nodes
    .filter((node) => (node.parentNodeId ?? null) === intent.parentNodeId)
    .sort((left, right) => left.position - right.position);
  const anchor = siblings(snapshot).slice(intent.position).find((node) => !moving.has(node.nodeId));
  let preview = snapshot;
  const operations: MoveOperation[] = [];
  for (const source of sources) {
    const node = preview.nodes.find((candidate) => candidate.nodeId === source.nodeId);
    if (!node) continue;
    const currentSiblings = siblings(preview);
    const position = anchor ? currentSiblings.findIndex((candidate) => candidate.nodeId === anchor.nodeId) : currentSiblings.length;
    const operation = workspaceTreeMoveOperation(node, { ...intent, position });
    operations.push(operation);
    preview = previewWorkspaceTreeMove(preview, operation);
  }
  return operations;
}

/** Render the released row while persistence is pending, without changing the store. */
export function previewWorkspaceTreeMove(
  snapshot: ProjectExplorerSnapshot,
  operation: MoveOperation,
): ProjectExplorerSnapshot {
  const nodes = snapshot.nodes.map((node) => ({ ...node }));
  const source = nodes.find((node) => node.nodeId === operation.nodeId);
  if (!source) return snapshot;
  const oldParent = source.parentNodeId ?? null;
  const nextParent = operation.parentNodeId ?? null;
  source.parentNodeId = nextParent;

  // Match move_node's sibling normalization, including hidden and pinned nodes.
  for (const parent of new Set([oldParent, nextParent])) {
    const siblings = nodes.filter((node) => node.nodeId !== source.nodeId
      && (node.parentNodeId ?? null) === parent)
      .sort((left, right) => left.position - right.position);
    if (parent === nextParent) {
      siblings.splice(Math.max(0, Math.min(operation.position, siblings.length)), 0, source);
    }
    siblings.forEach((node, position) => { node.position = position; });
  }

  return {
    ...snapshot,
    nodes,
    itemStates: snapshot.itemStates?.map((state) => (
      state.nodeId === source.nodeId && !state.relativePath
        ? { ...state, pinned: false }
        : state
    )),
  };
}
