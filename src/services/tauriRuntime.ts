import { getCurrentWindow } from "@tauri-apps/api/window";

type TauriInternals = {
  metadata?: {
    currentWindow?: {
      label?: string;
    };
  };
  invoke?: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
};

const WINDOW_DRAG_EXCLUDED_SELECTOR = [
  "button",
  "a",
  "input",
  "textarea",
  "select",
  "[contenteditable='true']",
  ".workspace-selector",
  ".window-controls",
  ".dir-dropdown",
  "[data-window-no-drag]",
].join(", ");

let windowDragFallbackInstalled = false;

function getTauriInternals(): TauriInternals | null {
  if (typeof window === "undefined") return null;
  const internals = (window as unknown as { __TAURI_INTERNALS__?: TauriInternals })
    .__TAURI_INTERNALS__;
  return internals ?? null;
}

export function hasTauriWindowRuntime(): boolean {
  const internals = getTauriInternals();
  return typeof internals?.invoke === "function";
}

export function getCurrentTauriWindowLabel(): string | null {
  if (!hasTauriWindowRuntime()) return null;
  try {
    return getCurrentWindow().label ?? null;
  } catch {
    return getTauriInternals()?.metadata?.currentWindow?.label ?? null;
  }
}

export async function showCurrentTauriWindow(): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  const window = getCurrentWindow();
  await window.show();
  await window.setFocus().catch(() => {
    /* Focusing can fail when the OS denies foreground activation. */
  });
}

export function startCurrentWindowDragging(): void {
  if (!hasTauriWindowRuntime()) return;
  getCurrentWindow().startDragging().catch((error) => {
    console.warn("Failed to start Tauri window drag:", error);
  });
}

export function canStartWindowDragFromTarget(target: EventTarget | null): boolean {
  if (typeof HTMLElement === "undefined" || !(target instanceof HTMLElement)) return false;
  return !target.closest(WINDOW_DRAG_EXCLUDED_SELECTOR);
}

function isCssWindowDragRegionTarget(target: EventTarget | null): boolean {
  if (typeof HTMLElement === "undefined" || !(target instanceof HTMLElement)) return false;
  let element: HTMLElement | null = target;
  while (element && element !== document.body) {
    const appRegion = window.getComputedStyle(element).getPropertyValue("-webkit-app-region").trim();
    if (appRegion === "no-drag") return false;
    if (appRegion === "drag") return true;
    element = element.parentElement;
  }
  return false;
}

export function installTauriWindowDragFallback(): void {
  if (windowDragFallbackInstalled || !hasTauriWindowRuntime()) return;
  windowDragFallbackInstalled = true;
  window.addEventListener("pointerdown", (event) => {
    if (event.defaultPrevented || event.button !== 0 || event.detail > 1) return;
    if (!canStartWindowDragFromTarget(event.target)) return;
    if (!isCssWindowDragRegionTarget(event.target)) return;
    event.preventDefault();
    startCurrentWindowDragging();
  });
}

export function toggleTauriDevtools(): void {
  const invoke = getTauriInternals()?.invoke;
  if (typeof invoke !== "function") return;
  void invoke("plugin:webview|internal_toggle_devtools").catch(() => {
    /* Devtools toggle is only available in debug builds. */
  });
}

export function installTauriDevtoolsHotkeys(): void {
  window.addEventListener("keydown", (event) => {
    if (event.key !== "F12") return;
    event.preventDefault();
    toggleTauriDevtools();
  });
}
