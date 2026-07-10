import { reactive } from "vue";

export type ShortcutAction = "newChat" | "cancelRun";
export type ShortcutPlatform = "default" | "mac";

export interface ShortcutDefinition {
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

export type ShortcutSettings = Record<ShortcutAction, ShortcutDefinition>;

const STORAGE_KEY = "locus-keyboard-shortcuts";

function normalizeKeyValue(value: string | null | undefined): string {
  const raw = String(value ?? "").trim();
  if (!raw) return "";
  if (raw === " ") return "space";
  const lower = raw.toLowerCase();
  if (lower === "spacebar") return "space";
  return lower;
}

function hasModifier(shortcut: ShortcutDefinition): boolean {
  return shortcut.ctrl || shortcut.meta || shortcut.alt || shortcut.shift;
}

function cloneShortcut(shortcut: ShortcutDefinition): ShortcutDefinition {
  return { ...shortcut };
}

function snapshotSettings(settings: ShortcutSettings): ShortcutSettings {
  return {
    newChat: cloneShortcut(settings.newChat),
    cancelRun: cloneShortcut(settings.cancelRun),
  };
}

export function detectShortcutPlatform(): ShortcutPlatform {
  if (typeof navigator !== "undefined" && /Mac|iPhone|iPod|iPad/i.test(navigator.platform)) {
    return "mac";
  }
  return "default";
}

export function createDefaultShortcutSettings(
  platform: ShortcutPlatform = detectShortcutPlatform(),
): ShortcutSettings {
  const useMeta = platform === "mac";
  return {
    newChat: {
      ctrl: !useMeta,
      meta: useMeta,
      alt: false,
      shift: false,
      key: "n",
    },
    cancelRun: {
      ctrl: false,
      meta: false,
      alt: false,
      shift: false,
      key: "escape",
    },
  };
}

function normalizeShortcut(
  shortcut: Partial<ShortcutDefinition> | null | undefined,
  fallback: ShortcutDefinition,
  requireModifier = true,
): ShortcutDefinition {
  const normalized: ShortcutDefinition = {
    ctrl: Boolean(shortcut?.ctrl),
    meta: Boolean(shortcut?.meta),
    alt: Boolean(shortcut?.alt),
    shift: Boolean(shortcut?.shift),
    key: normalizeKeyValue(shortcut?.key),
  };

  if (!normalized.key || (requireModifier && !hasModifier(normalized))) {
    return cloneShortcut(fallback);
  }

  return normalized;
}

function normalizeShortcutSettings(raw: unknown): ShortcutSettings {
  const defaults = createDefaultShortcutSettings();
  if (!raw || typeof raw !== "object") {
    return defaults;
  }

  const record = raw as Partial<Record<ShortcutAction, ShortcutDefinition>>;
  return {
    newChat: normalizeShortcut(record.newChat, defaults.newChat),
    cancelRun: normalizeShortcut(record.cancelRun, defaults.cancelRun, false),
  };
}

function loadShortcutSettings(): ShortcutSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      return normalizeShortcutSettings(JSON.parse(raw));
    }
  } catch {
    // ignore persistence failures
  }
  return createDefaultShortcutSettings();
}

function saveShortcutSettings(settings: ShortcutSettings) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshotSettings(settings)));
  } catch {
    // ignore persistence failures
  }
}

const state = reactive<ShortcutSettings>(loadShortcutSettings());

function keyLabel(key: string): string {
  switch (key) {
    case "space":
      return "Space";
    case "arrowup":
      return "Up";
    case "arrowdown":
      return "Down";
    case "arrowleft":
      return "Left";
    case "arrowright":
      return "Right";
    case "escape":
      return "Esc";
    case "backspace":
      return "Backspace";
    case "delete":
      return "Delete";
    default:
      if (key.length === 1) return key.toUpperCase();
      return key.charAt(0).toUpperCase() + key.slice(1);
  }
}

export function formatShortcutParts(
  shortcut: ShortcutDefinition,
  platform: ShortcutPlatform = detectShortcutPlatform(),
): string[] {
  const parts: string[] = [];
  if (shortcut.ctrl) parts.push("Ctrl");
  if (shortcut.alt) parts.push(platform === "mac" ? "Option" : "Alt");
  if (shortcut.shift) parts.push("Shift");
  if (shortcut.meta) parts.push(platform === "mac" ? "Cmd" : "Meta");
  parts.push(keyLabel(shortcut.key));
  return parts;
}

export function formatShortcut(
  shortcut: ShortcutDefinition,
  platform: ShortcutPlatform = detectShortcutPlatform(),
): string {
  return formatShortcutParts(shortcut, platform).join("+");
}

function isModifierKey(key: string): boolean {
  return key === "control" || key === "meta" || key === "alt" || key === "shift";
}

export function parseShortcutEvent(
  event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">,
  requireModifier = true,
): ShortcutDefinition | null {
  const key = normalizeKeyValue(event.key);
  if (!key || isModifierKey(key)) {
    return null;
  }

  const shortcut: ShortcutDefinition = {
    ctrl: Boolean(event.ctrlKey),
    meta: Boolean(event.metaKey),
    alt: Boolean(event.altKey),
    shift: Boolean(event.shiftKey),
    key,
  };

  if (requireModifier && !hasModifier(shortcut)) {
    return null;
  }

  return shortcut;
}

export function matchesShortcut(
  event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">,
  shortcut: ShortcutDefinition,
): boolean {
  const parsed = parseShortcutEvent(event, false);
  if (!parsed) return false;
  return parsed.ctrl === shortcut.ctrl
    && parsed.meta === shortcut.meta
    && parsed.alt === shortcut.alt
    && parsed.shift === shortcut.shift
    && parsed.key === shortcut.key;
}

export const DOUBLE_PRESS_SHORTCUT_INTERVAL_MS = 1_000;

export function createDoublePressShortcutTracker(
  intervalMs = DOUBLE_PRESS_SHORTCUT_INTERVAL_MS,
) {
  let firstPressedAt: number | null = null;

  return {
    press(now = Date.now()): boolean {
      const shouldTrigger = firstPressedAt !== null
        && now >= firstPressedAt
        && now - firstPressedAt <= intervalMs;
      firstPressedAt = shouldTrigger ? null : now;
      return shouldTrigger;
    },
    reset() {
      firstPressedAt = null;
    },
  };
}

export function useKeyboardShortcuts() {
  function setShortcut(action: ShortcutAction, shortcut: ShortcutDefinition) {
    state[action] = normalizeShortcut(
      shortcut,
      createDefaultShortcutSettings()[action],
      action !== "cancelRun",
    );
    saveShortcutSettings(state);
  }

  function resetShortcut(action: ShortcutAction) {
    const defaults = createDefaultShortcutSettings();
    state[action] = cloneShortcut(defaults[action]);
    saveShortcutSettings(state);
  }

  return {
    state,
    setShortcut,
    resetShortcut,
  };
}
