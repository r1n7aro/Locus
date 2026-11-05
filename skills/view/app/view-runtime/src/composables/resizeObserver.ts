export type ResizeObserverHandle = Pick<ResizeObserver, "observe" | "unobserve" | "disconnect">;

function requestObserverFrame(callback: FrameRequestCallback): number {
  if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
    return window.requestAnimationFrame(callback);
  }
  return setTimeout(() => callback(Date.now()), 0) as unknown as number;
}

function cancelObserverFrame(handle: number) {
  if (typeof window !== "undefined" && typeof window.cancelAnimationFrame === "function") {
    window.cancelAnimationFrame(handle);
    return;
  }
  clearTimeout(handle);
}

export interface AnimationFrameResizeObserverOptions {
  /**
   * Runs synchronously in the native observer callback, after layout and
   * before paint, for corrections that must land in the frame being drawn
   * (for example keeping a scroll position pinned). Keep it to cheap reads and
   * writes; `callback` is still delivered on the next animation frame.
   */
  beforePaint?: ResizeObserverCallback;
}

export function createAnimationFrameResizeObserver(
  callback: ResizeObserverCallback,
  options: AnimationFrameResizeObserverOptions = {},
): ResizeObserverHandle | null {
  if (typeof ResizeObserver === "undefined") return null;

  const pendingEntries = new Map<Element, ResizeObserverEntry>();
  let frame = 0;
  let observer: ResizeObserver;

  function cancelPendingFrame() {
    if (!frame) return;
    cancelObserverFrame(frame);
    frame = 0;
  }

  function flush() {
    frame = 0;
    if (pendingEntries.size === 0) return;
    const entries = Array.from(pendingEntries.values());
    pendingEntries.clear();
    callback(entries, observer);
  }

  observer = new ResizeObserver((entries) => {
    options.beforePaint?.(entries, observer);
    for (const entry of entries) {
      pendingEntries.set(entry.target, entry);
    }
    if (frame) return;
    frame = requestObserverFrame(flush);
  });

  return {
    observe(target, options) {
      observer.observe(target, options);
    },
    unobserve(target) {
      pendingEntries.delete(target);
      observer.unobserve(target);
    },
    disconnect() {
      cancelPendingFrame();
      pendingEntries.clear();
      observer.disconnect();
    },
  };
}
