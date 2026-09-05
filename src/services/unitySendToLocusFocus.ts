export type LastFocusedComposerTarget =
  | {
    surface: "workbench";
    windowId: string;
    checkoutId: string;
    paneId: string;
    editorId: string;
  }
  | {
    surface: "chatWorkspace";
    windowId: string;
    checkoutId: string;
  };

const LAST_FOCUSED_COMPOSER_STORAGE_KEY = "locus:last-focused-composer:v1";

export function readLastFocusedComposer(
  storage: Storage = window.localStorage,
): LastFocusedComposerTarget | null {
  const raw = storage.getItem(LAST_FOCUSED_COMPOSER_STORAGE_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<LastFocusedComposerTarget>;
    if (
      (parsed.surface !== "workbench" && parsed.surface !== "chatWorkspace")
      || typeof parsed.windowId !== "string"
      || typeof parsed.checkoutId !== "string"
    ) return null;
    if (
      parsed.surface === "workbench"
      && (typeof parsed.paneId !== "string" || typeof parsed.editorId !== "string")
    ) return null;
    return parsed as LastFocusedComposerTarget;
  } catch {
    return null;
  }
}

export function writeLastFocusedComposer(
  target: LastFocusedComposerTarget,
  storage: Storage = window.localStorage,
): void {
  storage.setItem(LAST_FOCUSED_COMPOSER_STORAGE_KEY, JSON.stringify(target));
}

export function clearLastFocusedComposer(
  target: Pick<LastFocusedComposerTarget, "surface" | "windowId">,
  storage: Storage = window.localStorage,
): void {
  const current = readLastFocusedComposer(storage);
  if (current?.surface === target.surface && current.windowId === target.windowId) {
    storage.removeItem(LAST_FOCUSED_COMPOSER_STORAGE_KEY);
  }
}
