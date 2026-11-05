// Pure keyboard shortcut helpers, shared by `useKeyboardShortcuts` and the
// settings legacy import (which must not depend on the settings store).

export type ShortcutAction = "newChat" | "cancelRun" | "findInContext" | "globalSearch";
export type ShortcutPlatform = "default" | "mac";

export interface ShortcutDefinition {
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

export type ShortcutSettings = Record<ShortcutAction, ShortcutDefinition>;

export const SHORTCUT_ACTIONS: readonly ShortcutAction[] = [
  "newChat",
  "cancelRun",
  "findInContext",
  "globalSearch",
];

/** Setting key holding each action's shortcut (`ctrl+shift+f`; empty = default). */
export const SHORTCUT_SETTING_KEYS: Record<ShortcutAction, string> = {
  newChat: "shortcuts.newChat",
  cancelRun: "shortcuts.cancelRun",
  findInContext: "shortcuts.findInContext",
  globalSearch: "shortcuts.globalSearch",
};

const MODIFIERS = ["ctrl", "meta", "alt", "shift"] as const;

export function normalizeKeyValue(value: string | null | undefined): string {
  const raw = String(value ?? "").trim();
  if (!raw) return "";
  if (raw === " ") return "space";
  const lower = raw.toLowerCase();
  if (lower === "spacebar") return "space";
  return lower;
}

export function hasModifier(shortcut: ShortcutDefinition): boolean {
  return shortcut.ctrl || shortcut.meta || shortcut.alt || shortcut.shift;
}

export function cloneShortcut(shortcut: ShortcutDefinition): ShortcutDefinition {
  return { ...shortcut };
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
    findInContext: { ctrl: !useMeta, meta: useMeta, alt: false, shift: false, key: "f" },
    globalSearch: { ctrl: !useMeta, meta: useMeta, alt: false, shift: true, key: "f" },
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

export function normalizeShortcut(
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

export function shortcutRequiresModifier(action: ShortcutAction): boolean {
  return action !== "cancelRun";
}

/** `ctrl+shift+f`; the stored form of a shortcut setting. */
export function shortcutToSetting(shortcut: ShortcutDefinition): string {
  const parts: string[] = MODIFIERS.filter((modifier) => shortcut[modifier]);
  parts.push(normalizeKeyValue(shortcut.key));
  return parts.join("+");
}

/** Parses a shortcut setting; `null` for an empty or malformed value. */
export function shortcutFromSetting(value: unknown): ShortcutDefinition | null {
  if (typeof value !== "string") return null;
  const text = value.trim().toLowerCase();
  if (!text) return null;
  let key: string;
  let modifiers: string[];
  if (text.endsWith("++")) {
    key = "+";
    modifiers = text.slice(0, -2).split("+");
  } else {
    const parts = text.split("+");
    key = parts.pop() ?? "";
    modifiers = parts;
  }
  modifiers = modifiers.filter(Boolean);
  if (!key || (MODIFIERS as readonly string[]).includes(key)) return null;
  if (modifiers.some((modifier) => !(MODIFIERS as readonly string[]).includes(modifier))) return null;
  return {
    ctrl: modifiers.includes("ctrl"),
    meta: modifiers.includes("meta"),
    alt: modifiers.includes("alt"),
    shift: modifiers.includes("shift"),
    key: normalizeKeyValue(key),
  };
}

export function sameShortcut(left: ShortcutDefinition, right: ShortcutDefinition): boolean {
  return shortcutToSetting(left) === shortcutToSetting(right);
}
