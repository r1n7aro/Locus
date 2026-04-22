import { describe, expect, it } from "vitest";
import {
  createDefaultShortcutSettings,
  formatShortcut,
  matchesShortcut,
  parseShortcutEvent,
  type ShortcutDefinition,
} from "../composables/useKeyboardShortcuts";

describe("keyboard shortcuts", () => {
  it("uses Ctrl+N as the default shortcut on non-mac platforms", () => {
    const defaults = createDefaultShortcutSettings("default");
    expect(formatShortcut(defaults.newChat, "default")).toBe("Ctrl+N");
  });

  it("uses Cmd+N as the default shortcut on macOS", () => {
    const defaults = createDefaultShortcutSettings("mac");
    expect(formatShortcut(defaults.newChat, "mac")).toBe("Cmd+N");
  });

  it("matches the exact modifier combination", () => {
    const shortcut: ShortcutDefinition = {
      ctrl: true,
      meta: false,
      alt: false,
      shift: false,
      key: "n",
    };

    expect(matchesShortcut({
      key: "n",
      ctrlKey: true,
      metaKey: false,
      altKey: false,
      shiftKey: false,
    }, shortcut)).toBe(true);

    expect(matchesShortcut({
      key: "n",
      ctrlKey: true,
      metaKey: false,
      altKey: false,
      shiftKey: true,
    }, shortcut)).toBe(false);
  });

  it("rejects shortcuts without modifier keys", () => {
    expect(parseShortcutEvent({
      key: "n",
      ctrlKey: false,
      metaKey: false,
      altKey: false,
      shiftKey: false,
    })).toBeNull();
  });
});
