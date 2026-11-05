/**
 * Top tab bar order for single-workspace mode. Workspace tree system nodes can
 * leave the tree and sit between the page tabs; the stored order lists both so
 * a node keeps the slot it was dropped into. Stored as
 * `workspace.tree.topBarNodes`, empty while every node stays in the tree.
 */

export const WORKSPACE_TOP_BAR_NODES_SETTING = "workspace.tree.topBarNodes";

/** Page tabs, in their default order. */
export const TOP_BAR_PAGE_IDS = ["development", "plugins", "settings"] as const;
/** System resources of the workspace tree that can move into the top bar. */
export const TOP_BAR_NODE_IDS = ["collaboration", "knowledge", "assets", "views", "agents", "archived"] as const;

export type TopBarPageId = typeof TOP_BAR_PAGE_IDS[number];
export type TopBarNodeId = typeof TOP_BAR_NODE_IDS[number];
export type TopBarEntryId = TopBarPageId | TopBarNodeId;

export function isTopBarPageId(value: unknown): value is TopBarPageId {
  return TOP_BAR_PAGE_IDS.includes(value as TopBarPageId);
}

export function isTopBarNodeId(value: unknown): value is TopBarNodeId {
  return TOP_BAR_NODE_IDS.includes(value as TopBarNodeId);
}

/** Full order: known entries of the stored value, with every page tab present. */
export function topBarLayout(value: unknown): TopBarEntryId[] {
  const layout: TopBarEntryId[] = [];
  for (const raw of Array.isArray(value) ? value : []) {
    const id = typeof raw === "string" ? raw.trim() : "";
    if ((isTopBarPageId(id) || isTopBarNodeId(id)) && !layout.includes(id)) layout.push(id);
  }
  TOP_BAR_PAGE_IDS.forEach((page, index) => {
    if (layout.includes(page)) return;
    // A page missing from an edited value returns beside its default neighbour.
    const previous = TOP_BAR_PAGE_IDS.slice(0, index).reverse().find((candidate) => layout.includes(candidate));
    layout.splice(previous ? layout.indexOf(previous) + 1 : 0, 0, page);
  });
  return layout;
}

export function topBarNodeIds(value: unknown): TopBarNodeId[] {
  return topBarLayout(value).filter(isTopBarNodeId);
}

function storedLayout(layout: TopBarEntryId[]): TopBarEntryId[] {
  return layout.some(isTopBarNodeId) ? layout : [];
}

/**
 * Places `node` in the top bar before `before`; `null` appends it. Without
 * `before`, a node already in the bar stays put and a new one follows the
 * Development tab and the nodes already beside it.
 */
export function placeTopBarNode(
  value: unknown,
  node: TopBarNodeId,
  before?: TopBarEntryId | null,
): TopBarEntryId[] {
  const layout = topBarLayout(value);
  if (before === node || (before === undefined && layout.includes(node))) return storedLayout(layout);
  const rest = layout.filter((id) => id !== node);
  let index: number;
  if (before === undefined) {
    index = rest.indexOf("development") + 1;
    while (index < rest.length && isTopBarNodeId(rest[index])) index += 1;
  } else {
    index = before === null ? -1 : rest.indexOf(before);
    if (index < 0) index = rest.length;
  }
  rest.splice(index, 0, node);
  return storedLayout(rest);
}

export function removeTopBarNode(value: unknown, node: TopBarNodeId): TopBarEntryId[] {
  return storedLayout(topBarLayout(value).filter((id) => id !== node));
}

export function sameTopBarLayout(left: unknown, right: unknown): boolean {
  const a = Array.isArray(left) ? left : [];
  const b = Array.isArray(right) ? right : [];
  return a.length === b.length && a.every((id, index) => id === b[index]);
}

/** The rendered tab a drop at `x` lands before, or `null` past the last tab. */
export function topBarInsertionBefore(
  x: number,
  tabs: ReadonlyArray<{ id: TopBarEntryId; left: number; right: number }>,
): TopBarEntryId | null {
  return tabs.find((tab) => x < (tab.left + tab.right) / 2)?.id ?? null;
}
