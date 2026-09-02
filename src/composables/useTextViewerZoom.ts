import { computed, readonly, ref, type CSSProperties } from "vue";

export const TEXT_VIEWER_ZOOM_STORAGE_KEY = "locus:textViewerFontScale";
export const TEXT_VIEWER_ZOOM_DEFAULT = 1;
export const TEXT_VIEWER_ZOOM_MIN = 0.7;
export const TEXT_VIEWER_ZOOM_MAX = 2;
export const TEXT_VIEWER_ZOOM_STEP = 0.1;

function roundZoomScale(value: number): number {
  return Math.round(value * 100) / 100;
}

export function normalizeTextViewerZoomScale(value: unknown): number {
  if (value === null || value === undefined || value === "") return TEXT_VIEWER_ZOOM_DEFAULT;
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed)) return TEXT_VIEWER_ZOOM_DEFAULT;
  return roundZoomScale(Math.min(TEXT_VIEWER_ZOOM_MAX, Math.max(TEXT_VIEWER_ZOOM_MIN, parsed)));
}

export function stepTextViewerZoomScale(current: number, deltaY: number): number {
  if (!Number.isFinite(deltaY) || deltaY === 0) return normalizeTextViewerZoomScale(current);
  const direction = deltaY < 0 ? 1 : -1;
  return normalizeTextViewerZoomScale(current + direction * TEXT_VIEWER_ZOOM_STEP);
}

function readStoredScale(): number {
  if (typeof localStorage === "undefined") return TEXT_VIEWER_ZOOM_DEFAULT;
  try {
    return normalizeTextViewerZoomScale(localStorage.getItem(TEXT_VIEWER_ZOOM_STORAGE_KEY));
  } catch {
    return TEXT_VIEWER_ZOOM_DEFAULT;
  }
}

const scale = ref(readStoredScale());
const style = computed<CSSProperties>(() => ({
  "--text-viewer-font-scale": String(scale.value),
}));

function persistScale(value: number): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(TEXT_VIEWER_ZOOM_STORAGE_KEY, String(value));
  } catch {
    // Font zoom remains available for the current window when storage is unavailable.
  }
}

export function resetTextViewerZoomScale(): void {
  scale.value = TEXT_VIEWER_ZOOM_DEFAULT;
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.removeItem(TEXT_VIEWER_ZOOM_STORAGE_KEY);
  } catch {
    // The in-memory reset still applies when storage is unavailable.
  }
}

function handleWheel(event: WheelEvent): boolean {
  if (!event.ctrlKey && !event.metaKey) return false;
  event.preventDefault();
  if (event.deltaY === 0) return true;
  const next = stepTextViewerZoomScale(scale.value, event.deltaY);
  if (next === scale.value) return true;
  scale.value = next;
  persistScale(next);
  return true;
}

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (event.key !== null && event.key !== TEXT_VIEWER_ZOOM_STORAGE_KEY) return;
    scale.value = readStoredScale();
  });
}

export function useTextViewerZoom() {
  return {
    textViewerZoomScale: readonly(scale),
    textViewerZoomStyle: style,
    handleTextViewerZoomWheel: handleWheel,
  };
}
