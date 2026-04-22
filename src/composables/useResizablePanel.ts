import { onMounted, onUnmounted, ref } from "vue";
import { acquireSelectionLock } from "./useSelectionLock";

export interface ResizablePanelOptions {
  storageKey: string;
  defaultSize: number;
  minSize: number | ((container: HTMLElement) => number);
  maxSize: number | ((container: HTMLElement) => number);
  direction?: "horizontal" | "vertical";
}

export function useResizablePanel(containerRef: { value: HTMLElement | null }, options: ResizablePanelOptions) {
  const size = ref(options.defaultSize);
  const isDragging = ref(false);
  let releaseSelectionLock: (() => void) | null = null;

  function resolveConstraint(
    value: ResizablePanelOptions["minSize"] | ResizablePanelOptions["maxSize"],
    fallback: number,
  ) {
    const container = containerRef.value;
    if (!container) return fallback;
    return typeof value === "function" ? value(container) : value;
  }

  function clampSize(next: number) {
    const minSize = resolveConstraint(options.minSize, options.defaultSize);
    const maxSize = resolveConstraint(options.maxSize, options.defaultSize);
    const boundedMin = Math.min(minSize, maxSize);
    const boundedMax = Math.max(minSize, maxSize);
    return Math.max(boundedMin, Math.min(boundedMax, next));
  }

  function stopDragging(persist: boolean) {
    if (!isDragging.value && !releaseSelectionLock) return;
    isDragging.value = false;
    document.removeEventListener("mousemove", onMouseMove);
    document.removeEventListener("mouseup", onMouseUp);
    document.body.style.cursor = "";
    releaseSelectionLock?.();
    releaseSelectionLock = null;
    if (!persist) return;
    try {
      localStorage.setItem(options.storageKey, String(Math.round(size.value)));
    } catch {
      // ignore persistence failures
    }
  }

  function onMouseDown(e: MouseEvent) {
    e.preventDefault();
    stopDragging(false);
    isDragging.value = true;
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    document.body.style.cursor = options.direction === "vertical" ? "row-resize" : "col-resize";
    releaseSelectionLock?.();
    releaseSelectionLock = acquireSelectionLock();
  }

  function onMouseMove(e: MouseEvent) {
    if (!isDragging.value || !containerRef.value) return;
    const rect = containerRef.value.getBoundingClientRect();
    const pos = options.direction === "vertical"
      ? e.clientY - rect.top
      : e.clientX - rect.left;
    size.value = clampSize(pos);
  }

  function onMouseUp() {
    stopDragging(true);
  }

  function onWindowResize() {
    size.value = clampSize(size.value);
  }

  onMounted(() => {
    try {
      const saved = localStorage.getItem(options.storageKey);
      if (saved) {
        size.value = clampSize(Number(saved));
      }
    } catch {
      // ignore persistence failures
    }
    size.value = clampSize(size.value);
    window.addEventListener("resize", onWindowResize);
  });

  onUnmounted(() => {
    window.removeEventListener("resize", onWindowResize);
    stopDragging(false);
  });

  return { size, isDragging, onMouseDown };
}
