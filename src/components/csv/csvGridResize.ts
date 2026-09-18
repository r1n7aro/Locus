import { createAnimationFrameResizeObserver } from "../../composables/resizeObserver";

// Splitters and sidebar animations change the viewport on successive frames.
const RESIZE_SETTLE_MS = 80;

export function observeCsvGridResize(element: HTMLElement, redraw: () => void, canRedraw: () => boolean) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  let pending = false;
  let width = -1;
  let height = -1;

  function pause(): void {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  }
  function resume(): void {
    if (disposed || !pending || timer !== undefined || !canRedraw()) return;
    timer = setTimeout(() => {
      timer = undefined;
      if (disposed || !pending || !canRedraw() || width <= 0 || height <= 0) return;
      pending = false;
      redraw();
    }, RESIZE_SETTLE_MS);
  }
  const observer = createAnimationFrameResizeObserver((entries) => {
    const entry = entries.find((entry) => entry.target === element);
    if (!entry || disposed) return;
    const nextWidth = Math.floor(entry.contentRect.width);
    const nextHeight = Math.floor(entry.contentRect.height);
    if (width === nextWidth && height === nextHeight) return;
    const initial = width < 0;
    width = nextWidth; height = nextHeight;
    // The initial visible table is already laid out by its data load.
    // A table first observed at zero size is redrawn when it becomes visible.
    pending ||= !initial;
    pause();
    resume();
  });
  observer?.observe(element);

  return {
    pause,
    resume,
    request() { pending = true; resume(); },
    disconnect() { disposed = true; pause(); observer?.disconnect(); },
  };
}
