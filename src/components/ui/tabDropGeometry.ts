export interface TabDropBounds {
  left: number;
  right: number;
}

/** Resolve an insertion position from the half of each visible tab under the pointer. */
export function tabInsertionIndexAtPoint(
  clientX: number,
  tabs: readonly TabDropBounds[],
): number {
  const before = tabs.findIndex((tab) => clientX < tab.left + (tab.right - tab.left) / 2);
  return before >= 0 ? before : tabs.length;
}

/** Move one item using a pre-removal insertion index. */
export function moveTabAtInsertionIndex<T>(
  items: readonly T[],
  fromIndex: number,
  insertionIndex: number,
): T[] {
  if (fromIndex < 0 || fromIndex >= items.length) return [...items];
  const boundedInsertion = Math.min(items.length, Math.max(0, insertionIndex));
  const next = [...items];
  const [item] = next.splice(fromIndex, 1);
  const adjustedInsertion = boundedInsertion > fromIndex
    ? boundedInsertion - 1
    : boundedInsertion;
  next.splice(adjustedInsertion, 0, item);
  return next;
}
