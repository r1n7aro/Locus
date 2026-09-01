import {
  LogicalPosition,
  LogicalSize,
  getCurrentWindow,
} from "@tauri-apps/api/window";

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
  ".window-controls",
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

export async function setCurrentTauriWindowBounds(bounds: {
  x: number;
  y: number;
  width: number;
  height: number;
}): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  const currentWindow = getCurrentWindow();
  await currentWindow.unminimize().catch(() => undefined);
  await Promise.all([
    currentWindow.setPosition(new LogicalPosition(bounds.x, bounds.y)),
    currentWindow.setSize(new LogicalSize(bounds.width, bounds.height)),
  ]);
  await currentWindow.show();
  await currentWindow.setFocus().catch(() => undefined);
}

export async function getCurrentTauriWindowPhysicalBounds(): Promise<{
  x: number;
  y: number;
  width: number;
  height: number;
}> {
  const currentWindow = getCurrentWindow();
  const [position, size] = await Promise.all([
    currentWindow.outerPosition(),
    currentWindow.outerSize(),
  ]);
  return {
    x: position.x,
    y: position.y,
    width: size.width,
    height: size.height,
  };
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

export function toggleTauriDevtools(): Promise<void> {
  const invoke = getTauriInternals()?.invoke;
  if (typeof invoke !== "function") return Promise.resolve();
  return invoke("plugin:webview|internal_toggle_devtools").then(() => undefined);
}

type DevtoolsAccessResolver = () => boolean | Promise<boolean>;

function isDevtoolsHotkey(event: KeyboardEvent): boolean {
  if (event.key === "F12") return true;
  if (event.code !== "KeyI") return false;
  return (event.ctrlKey && event.shiftKey) || (event.metaKey && event.altKey);
}

export function installTauriDevtoolsHotkeys(
  canToggleDevtools: DevtoolsAccessResolver = () => true,
): void {
  window.addEventListener("keydown", (event) => {
    if (!isDevtoolsHotkey(event)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    void Promise.resolve(canToggleDevtools())
      .then((enabled) => {
        if (!enabled) return;
        return toggleTauriDevtools();
      })
      .catch(() => {
        /* Debug mode or the release DevTools capability may be unavailable. */
      });
  }, true);
}
