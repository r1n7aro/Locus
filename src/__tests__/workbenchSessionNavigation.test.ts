import { describe, expect, it } from "vitest";
import {
  workbenchNewSessionShortcutAction,
  workbenchSessionNavigationMode,
} from "../components/workbench/workbenchSessionNavigation";

describe("workbench session navigation", () => {
  it("activates an existing session before considering placement", () => {
    expect(workbenchSessionNavigationMode({
      targetOpen: true,
      splitLayout: true,
      focusedGroupTabCount: 3,
    })).toBe("activate");
  });

  it("reuses the only hidden tab for an ordinary single click", () => {
    expect(workbenchSessionNavigationMode({
      targetOpen: false,
      splitLayout: false,
      focusedGroupTabCount: 1,
    })).toBe("reuse");
  });

  it.each([
    ["split layout", { splitLayout: true, focusedGroupTabCount: 1 }],
    ["visible tab strip", { splitLayout: false, focusedGroupTabCount: 2 }],
    ["protected editor", {
      splitLayout: false,
      focusedGroupTabCount: 1,
      currentEditorProtected: true,
    }],
    ["empty group", { splitLayout: false, focusedGroupTabCount: 0 }],
  ])("creates a tab for %s", (_label, context) => {
    expect(workbenchSessionNavigationMode({
      targetOpen: false,
      ...context,
    })).toBe("newTab");
  });
});

describe("workbench new-session shortcut", () => {
  it("opens a new tab when the tab strip is already visible", () => {
    expect(workbenchNewSessionShortcutAction({
      currentIsNewSession: false,
      tabStripVisible: true,
    })).toBe("newTab");
  });

  it("keeps the current tab when it already represents a new session", () => {
    expect(workbenchNewSessionShortcutAction({
      currentIsNewSession: true,
      tabStripVisible: true,
    })).toBe("keepCurrent");
  });

  it("replaces the single hidden tab when tab mode is not visible", () => {
    expect(workbenchNewSessionShortcutAction({
      currentIsNewSession: false,
      tabStripVisible: false,
    })).toBe("replaceCurrent");
  });
});
