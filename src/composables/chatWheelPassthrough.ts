function resolveWheelDelta(event: WheelEvent, scrollTarget: HTMLElement): number {
  switch (event.deltaMode) {
    case WheelEvent.DOM_DELTA_LINE:
      return event.deltaY * 16;
    case WheelEvent.DOM_DELTA_PAGE:
      return event.deltaY * scrollTarget.clientHeight;
    default:
      return event.deltaY;
  }
}

function isVerticallyScrollable(element: HTMLElement): boolean {
  const style = window.getComputedStyle(element);
  const overflowY = style.overflowY;
  return (overflowY === "auto" || overflowY === "scroll" || overflowY === "overlay")
    && element.scrollHeight > element.clientHeight;
}

function canConsumeVerticalScroll(element: HTMLElement, deltaY: number): boolean {
  if (!isVerticallyScrollable(element)) return false;
  if (deltaY < 0) return element.scrollTop > 0;
  if (deltaY > 0) return element.scrollTop + element.clientHeight < element.scrollHeight;
  return false;
}

function findScrollableAncestorWithin(
  target: EventTarget | null,
  boundary: HTMLElement | null,
  deltaY: number,
): HTMLElement | null {
  let current =
    target instanceof HTMLElement
      ? target
      : target instanceof Node
        ? target.parentElement
        : null;

  while (current && current !== boundary) {
    if (canConsumeVerticalScroll(current, deltaY)) {
      return current;
    }
    current = current.parentElement;
  }

  return null;
}

export function forwardWheelToElement(event: WheelEvent, scrollTarget: HTMLElement | null): void {
  if (!scrollTarget || event.defaultPrevented || event.ctrlKey) return;

  const deltaY = resolveWheelDelta(event, scrollTarget);
  if (!deltaY) return;

  const boundary = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  if (findScrollableAncestorWithin(event.target, boundary, deltaY)) {
    return;
  }

  const maxScrollTop = scrollTarget.scrollHeight - scrollTarget.clientHeight;
  if (maxScrollTop <= 0) return;

  const nextScrollTop = Math.max(0, Math.min(maxScrollTop, scrollTarget.scrollTop + deltaY));
  if (nextScrollTop === scrollTarget.scrollTop) return;

  event.preventDefault();
  scrollTarget.scrollTop = nextScrollTop;
}
