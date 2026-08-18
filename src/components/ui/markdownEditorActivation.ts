export interface MarkdownEditorActivationPoint {
  x: number;
  y: number;
}

interface MarkdownEditorScrollPosition {
  element: HTMLElement;
  top: number;
  left: number;
}

export interface MarkdownEditorActivationSnapshot {
  point: MarkdownEditorActivationPoint | null;
  scrollPositions: MarkdownEditorScrollPosition[];
  renderedHeight: number;
}

const SCROLLABLE_OVERFLOW_PATTERN = /^(auto|scroll|overlay)$/;

function isScrollableElement(element: HTMLElement): boolean {
  if (element.scrollTop !== 0 || element.scrollLeft !== 0) return true;
  const view = element.ownerDocument.defaultView;
  if (!view) return false;
  const style = view.getComputedStyle(element);
  return SCROLLABLE_OVERFLOW_PATTERN.test(style.overflowY)
    || SCROLLABLE_OVERFLOW_PATTERN.test(style.overflowX)
    || SCROLLABLE_OVERFLOW_PATTERN.test(style.overflow);
}

function captureScrollableAncestors(source: HTMLElement): MarkdownEditorScrollPosition[] {
  const positions: MarkdownEditorScrollPosition[] = [];
  const seen = new Set<HTMLElement>();
  let current = source.parentElement;

  while (current) {
    if (isScrollableElement(current)) {
      positions.push({
        element: current,
        top: current.scrollTop,
        left: current.scrollLeft,
      });
      seen.add(current);
    }
    current = current.parentElement;
  }

  const documentScroller = source.ownerDocument.scrollingElement;
  if (documentScroller instanceof HTMLElement && !seen.has(documentScroller)) {
    positions.push({
      element: documentScroller,
      top: documentScroller.scrollTop,
      left: documentScroller.scrollLeft,
    });
  }

  return positions;
}

export function captureMarkdownEditorActivation(
  source: HTMLElement,
  point: MarkdownEditorActivationPoint | null,
): MarkdownEditorActivationSnapshot {
  return {
    point,
    scrollPositions: captureScrollableAncestors(source),
    renderedHeight: source.getBoundingClientRect().height,
  };
}

export function restoreMarkdownEditorActivationScroll(
  snapshot: MarkdownEditorActivationSnapshot,
): void {
  for (const position of snapshot.scrollPositions) {
    position.element.scrollTop = position.top;
    position.element.scrollLeft = position.left;
  }
}

function caretRangeFromPoint(
  document: Document,
  point: MarkdownEditorActivationPoint,
): Range | null {
  const caretPosition = document.caretPositionFromPoint?.(point.x, point.y);
  if (caretPosition) {
    const range = document.createRange();
    try {
      range.setStart(caretPosition.offsetNode, caretPosition.offset);
      range.collapse(true);
      return range;
    } catch {
      return null;
    }
  }

  return document.caretRangeFromPoint?.(point.x, point.y) ?? null;
}

export function placeMarkdownEditorCaretAtActivation(
  editable: HTMLElement,
  snapshot: MarkdownEditorActivationSnapshot,
): boolean {
  if (!snapshot.point) return false;
  const document = editable.ownerDocument;
  const range = caretRangeFromPoint(document, snapshot.point);
  if (!range || (!editable.isSameNode(range.startContainer) && !editable.contains(range.startContainer))) {
    return false;
  }

  const selection = document.getSelection();
  if (!selection) return false;
  selection.removeAllRanges();
  selection.addRange(range);
  return true;
}

export function focusMarkdownEditorAtActivation(
  editable: HTMLElement,
  snapshot: MarkdownEditorActivationSnapshot,
): void {
  try {
    editable.focus({ preventScroll: true });
  } catch {
    editable.focus();
  }
  restoreMarkdownEditorActivationScroll(snapshot);
  placeMarkdownEditorCaretAtActivation(editable, snapshot);
  restoreMarkdownEditorActivationScroll(snapshot);
}
