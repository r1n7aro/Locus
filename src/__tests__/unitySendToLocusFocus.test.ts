// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";
import {
  clearLastFocusedComposer,
  readLastFocusedComposer,
  writeLastFocusedComposer,
} from "../services/unitySendToLocusFocus";

beforeEach(() => {
  window.localStorage.clear();
});

describe("Send to Locus composer focus", () => {
  it("stores workbench and standalone chat targets", () => {
    writeLastFocusedComposer({
      surface: "workbench",
      windowId: "main",
      checkoutId: "checkout-a",
      paneId: "main",
      editorId: "session-1",
    });
    expect(readLastFocusedComposer()).toEqual({
      surface: "workbench",
      windowId: "main",
      checkoutId: "checkout-a",
      paneId: "main",
      editorId: "session-1",
    });

    writeLastFocusedComposer({
      surface: "chatWorkspace",
      windowId: "chat-session-1",
      checkoutId: "checkout-a",
    });
    expect(readLastFocusedComposer()).toEqual({
      surface: "chatWorkspace",
      windowId: "chat-session-1",
      checkoutId: "checkout-a",
    });
  });

  it("clears only the matching composer surface", () => {
    writeLastFocusedComposer({
      surface: "chatWorkspace",
      windowId: "chat-session-1",
      checkoutId: "checkout-a",
    });
    clearLastFocusedComposer({ surface: "workbench", windowId: "main" });
    expect(readLastFocusedComposer()).not.toBeNull();

    clearLastFocusedComposer({ surface: "chatWorkspace", windowId: "chat-session-1" });
    expect(readLastFocusedComposer()).toBeNull();
  });
});
