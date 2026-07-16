export interface SelectionRectLike {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}

export interface SelectionLike {
  isCollapsed: boolean;
  rangeCount: number;
  toString(): string;
  getRangeAt(index: number): { getClientRects(): ArrayLike<SelectionRectLike> };
}

export const SELECTION_CONTEXT_HIT_TOLERANCE_PX = 3;

/**
 * Returns the selected text when `point` (viewport coordinates) falls on the
 * rendered selection highlight — mirroring native context-menu semantics where
 * only a right click on the highlight targets the selection. Returns "" for
 * collapsed/whitespace-only selections or clicks outside the highlight.
 */
export function selectionTextAtPoint(
  selection: SelectionLike | null | undefined,
  point: { x: number; y: number },
  tolerance: number = SELECTION_CONTEXT_HIT_TOLERANCE_PX,
): string {
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return "";
  const text = selection.toString();
  if (!text.trim()) return "";
  for (let rangeIndex = 0; rangeIndex < selection.rangeCount; rangeIndex += 1) {
    const rects = selection.getRangeAt(rangeIndex).getClientRects();
    for (let rectIndex = 0; rectIndex < rects.length; rectIndex += 1) {
      const rect = rects[rectIndex];
      if (!rect || rect.width <= 0 || rect.height <= 0) continue;
      if (
        point.x >= rect.left - tolerance
        && point.x <= rect.right + tolerance
        && point.y >= rect.top - tolerance
        && point.y <= rect.bottom + tolerance
      ) {
        return text;
      }
    }
  }
  return "";
}
