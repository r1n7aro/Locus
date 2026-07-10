import { describe, expect, it } from "vitest";
import {
  createDoublePressShortcutTracker,
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

  it("uses Esc as the default stop-response shortcut", () => {
    const defaults = createDefaultShortcutSettings("default");
    expect(formatShortcut(defaults.cancelRun, "default")).toBe("Esc");
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

  it("matches a configured single-key shortcut", () => {
    const shortcut: ShortcutDefinition = {
      ctrl: false,
      meta: false,
      alt: false,
      shift: false,
      key: "escape",
    };

    expect(matchesShortcut({
      key: "Escape",
      ctrlKey: false,
      metaKey: false,
      altKey: false,
      shiftKey: false,
    }, shortcut)).toBe(true);
  });

  it("triggers a double-press shortcut only inside the interval", () => {
    const tracker = createDoublePressShortcutTracker(1_000);

    expect(tracker.press(100)).toBe(false);
    expect(tracker.press(1_100)).toBe(true);
    expect(tracker.press(2_000)).toBe(false);
    expect(tracker.press(3_001)).toBe(false);
    expect(tracker.press(3_500)).toBe(true);
    tracker.press(4_000);
    tracker.reset();
    expect(tracker.press(4_100)).toBe(false);
  });
});
