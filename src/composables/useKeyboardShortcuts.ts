import { reactive, watch } from "vue";
import {
  SHORTCUT_ACTIONS,
  SHORTCUT_SETTING_KEYS,
  cloneShortcut,
  createDefaultShortcutSettings,
  detectShortcutPlatform,
  hasModifier,
  normalizeKeyValue,
  normalizeShortcut,
  sameShortcut,
  shortcutFromSetting,
  shortcutRequiresModifier,
  shortcutToSetting,
  type ShortcutAction,
  type ShortcutDefinition,
  type ShortcutPlatform,
  type ShortcutSettings,
} from "./keyboardShortcutCore";
import { saveSetting, settingValue } from "./useSettingsStore";

export {
  createDefaultShortcutSettings,
  detectShortcutPlatform,
  type ShortcutAction,
  type ShortcutDefinition,
  type ShortcutPlatform,
  type ShortcutSettings,
} from "./keyboardShortcutCore";

/** Each shortcut setting holds `ctrl+shift+f`; empty follows the platform default. */
function project(): ShortcutSettings {
  const defaults = createDefaultShortcutSettings();
  return Object.fromEntries(SHORTCUT_ACTIONS.map((action) => [
    action,
    normalizeShortcut(
      shortcutFromSetting(settingValue(SHORTCUT_SETTING_KEYS[action])),
      defaults[action],
      shortcutRequiresModifier(action),
    ),
  ])) as ShortcutSettings;
}

const state = reactive<ShortcutSettings>(project());
watch(project, (next) => {
  for (const action of SHORTCUT_ACTIONS) {
    if (!sameShortcut(state[action], next[action])) state[action] = next[action];
  }
}, { flush: "sync" });

/** Stores a shortcut; the platform default is stored as empty. */
function saveShortcut(action: ShortcutAction, shortcut: ShortcutDefinition) {
  const isDefault = sameShortcut(shortcut, createDefaultShortcutSettings()[action]);
  saveSetting(SHORTCUT_SETTING_KEYS[action], isDefault ? "" : shortcutToSetting(shortcut));
}

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
    saveShortcut(action, normalizeShortcut(
      shortcut,
      createDefaultShortcutSettings()[action],
      shortcutRequiresModifier(action),
    ));
  }

  function resetShortcut(action: ShortcutAction) {
    saveShortcut(action, cloneShortcut(createDefaultShortcutSettings()[action]));
  }

  return {
    state,
    setShortcut,
    resetShortcut,
  };
}
