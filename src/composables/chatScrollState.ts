export interface ScrollMetrics {
  scrollTop: number;
  clientHeight: number;
  scrollHeight: number;
}

export interface ScrollAnchorSnapshot {
  anchorId: string;
  offsetTop: number;
  fallbackScrollTop: number;
}

export interface LiveScrollAnchorSnapshot {
  anchor: HTMLElement;
  offsetTop: number;
  fallbackScrollTop: number;
}

export type SessionScrollState =
  | { mode: "bottom" }
  | { mode: "anchor"; anchorId: string; offsetTop: number; fallbackScrollTop: number }
  | { mode: "offset"; scrollTop: number };

export const CHAT_SCROLL_BOTTOM_THRESHOLD = 48;
export const CHAT_SCROLL_ANCHOR_SELECTOR = "[data-scroll-anchor-id]";

export function isNearBottom(
  metrics: ScrollMetrics,
  threshold = CHAT_SCROLL_BOTTOM_THRESHOLD,
): boolean {
  return metrics.scrollHeight - (metrics.scrollTop + metrics.clientHeight) <= threshold;
}

export function captureSessionScrollState(
  metrics: ScrollMetrics,
  anchor: ScrollAnchorSnapshot | null = null,
  threshold = CHAT_SCROLL_BOTTOM_THRESHOLD,
): SessionScrollState {
  if (isNearBottom(metrics, threshold)) {
    return { mode: "bottom" };
  }

  if (anchor?.anchorId) {
    return {
      mode: "anchor",
      anchorId: anchor.anchorId,
      offsetTop: anchor.offsetTop,
      fallbackScrollTop: Math.max(0, anchor.fallbackScrollTop),
    };
  }

  return { mode: "offset", scrollTop: Math.max(0, metrics.scrollTop) };
}

export function resolveSessionScrollTop(
  metrics: ScrollMetrics,
  state: SessionScrollState | null | undefined,
): number {
  const maxScrollTop = Math.max(0, metrics.scrollHeight - metrics.clientHeight);
  if (!state || state.mode === "bottom") {
    return maxScrollTop;
  }

  if (state.mode === "anchor") {
    return Math.max(0, Math.min(state.fallbackScrollTop, maxScrollTop));
  }

  return Math.max(0, Math.min(state.scrollTop, maxScrollTop));
}

function iterScrollAnchors(
  container: Pick<HTMLElement, "querySelectorAll">,
  selector = CHAT_SCROLL_ANCHOR_SELECTOR,
): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(selector));
}

export function captureScrollAnchor(
  container: Pick<HTMLElement, "scrollTop" | "getBoundingClientRect" | "querySelectorAll">,
  selector = CHAT_SCROLL_ANCHOR_SELECTOR,
): ScrollAnchorSnapshot | null {
  const containerRect = container.getBoundingClientRect();
  const anchors = iterScrollAnchors(container, selector);

  for (const anchor of anchors) {
    const anchorId = anchor.dataset.scrollAnchorId?.trim();
    if (!anchorId) continue;

    const rect = anchor.getBoundingClientRect();
    if (rect.height <= 0) continue;
    if (rect.bottom <= containerRect.top) continue;

    return {
      anchorId,
      offsetTop: rect.top - containerRect.top,
      fallbackScrollTop: Math.max(0, container.scrollTop),
    };
  }

  return null;
}

export function restoreScrollAnchor(
  container: Pick<
    HTMLElement,
    "scrollTop" | "scrollHeight" | "clientHeight" | "getBoundingClientRect" | "querySelectorAll"
  >,
  state: SessionScrollState | null | undefined,
  selector = CHAT_SCROLL_ANCHOR_SELECTOR,
): boolean {
  if (!state || state.mode !== "anchor" || !state.anchorId) return false;

  const anchor = iterScrollAnchors(container, selector)
    .find((candidate) => candidate.dataset.scrollAnchorId === state.anchorId);
  if (!anchor) return false;

  const containerRect = container.getBoundingClientRect();
  const anchorRect = anchor.getBoundingClientRect();
  const delta = (anchorRect.top - containerRect.top) - state.offsetTop;
  if (Math.abs(delta) < 0.5) return true;

  const maxScrollTop = Math.max(0, container.scrollHeight - container.clientHeight);
  const nextScrollTop = Math.max(0, Math.min(maxScrollTop, container.scrollTop + delta));
  if (nextScrollTop === container.scrollTop) return true;

  container.scrollTop = nextScrollTop;
  return true;
}

export function captureLiveScrollAnchor(
  container: Pick<HTMLElement, "scrollTop" | "getBoundingClientRect"> & Partial<Pick<HTMLElement, "contains">>,
  anchor: HTMLElement | null | undefined,
): LiveScrollAnchorSnapshot | null {
  if (!anchor) return null;
  if (container.contains && !container.contains(anchor)) return null;

  const containerRect = container.getBoundingClientRect();
  const anchorRect = anchor.getBoundingClientRect();
  return {
    anchor,
    offsetTop: anchorRect.top - containerRect.top,
    fallbackScrollTop: Math.max(0, container.scrollTop),
  };
}

export function restoreLiveScrollAnchor(
  container: Pick<
    HTMLElement,
    "scrollTop" | "scrollHeight" | "clientHeight" | "getBoundingClientRect"
  > & Partial<Pick<HTMLElement, "contains">>,
  state: LiveScrollAnchorSnapshot | null | undefined,
): boolean {
  if (!state?.anchor) return false;
  if (container.contains && !container.contains(state.anchor)) return false;

  const containerRect = container.getBoundingClientRect();
  const anchorRect = state.anchor.getBoundingClientRect();
  const delta = (anchorRect.top - containerRect.top) - state.offsetTop;
  if (Math.abs(delta) < 0.5) return true;

  const maxScrollTop = Math.max(0, container.scrollHeight - container.clientHeight);
  const nextScrollTop = Math.max(0, Math.min(maxScrollTop, container.scrollTop + delta));
  if (nextScrollTop === container.scrollTop) return true;

  container.scrollTop = nextScrollTop;
  return true;
}
