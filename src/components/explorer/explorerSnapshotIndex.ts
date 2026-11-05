import type { ProjectExplorerItemState, ProjectExplorerNode } from "../../types/workbench";

// Explorer snapshots are replaced as a whole and never mutated in place, so the
// arrays themselves key the indexes. Keys are the arrays as the caller sees them
// (reactive proxy or raw), so indexed nodes keep the identity the caller would
// get by iterating that array.
const childrenIndexes = new WeakMap<readonly ProjectExplorerNode[], Map<string | null, ProjectExplorerNode[]>>();
const itemStateIndexes = new WeakMap<readonly ProjectExplorerItemState[], Map<string, ProjectExplorerItemState>>();
const EMPTY_NODES: readonly ProjectExplorerNode[] = [];

/** Snapshot nodes grouped by `parentNodeId` (root under `null`), in snapshot order. */
export function explorerChildNodes(
  nodes: readonly ProjectExplorerNode[],
  parentNodeId: string | null,
): readonly ProjectExplorerNode[] {
  let index = childrenIndexes.get(nodes);
  if (!index) {
    index = new Map();
    for (const node of nodes) {
      const parent = node.parentNodeId ?? null;
      const siblings = index.get(parent);
      if (siblings) siblings.push(node);
      else index.set(parent, [node]);
    }
    childrenIndexes.set(nodes, index);
  }
  return index.get(parentNodeId) ?? EMPTY_NODES;
}

// A state's empty relative path never matches a lookup (lookups map "" to null).
function stateKey(nodeId: string, relativePath: string | null | undefined): string {
  return relativePath == null ? `${nodeId}\u0000` : `${nodeId}\u0000\u0001${relativePath}`;
}

/** First item state for the node and relative path, as `itemStates.find(...)` would return. */
export function indexedExplorerItemState(
  itemStates: readonly ProjectExplorerItemState[],
  nodeId: string,
  relativePath?: string | null,
): ProjectExplorerItemState | undefined {
  let index = itemStateIndexes.get(itemStates);
  if (!index) {
    index = new Map();
    for (const state of itemStates) {
      const key = stateKey(state.nodeId, state.relativePath);
      if (!index.has(key)) index.set(key, state);
    }
    itemStateIndexes.set(itemStates, index);
  }
  return index.get(stateKey(nodeId, relativePath || null));
}
