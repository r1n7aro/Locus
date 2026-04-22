import { describe, expect, it } from "vitest";
import {
  getChatSubmitModifierLabel,
  shouldSelectPopupOnEnter,
  shouldSubmitOnEnter,
} from "../composables/useChatInputSettings";

describe("chat input settings", () => {
  it("submits on plain Enter in enter-send mode", () => {
    expect(shouldSubmitOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    }, "enter-send")).toBe(true);

    expect(shouldSubmitOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: true,
      metaKey: false,
    }, "enter-send")).toBe(false);
  });

  it("submits on Ctrl/Cmd+Enter in modifier-send mode", () => {
    expect(shouldSubmitOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: true,
      metaKey: false,
    }, "mod-enter-send")).toBe(true);

    expect(shouldSubmitOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    }, "mod-enter-send")).toBe(false);
  });

  it("ignores Enter shortcuts during IME composition", () => {
    expect(shouldSubmitOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
      isComposing: true,
    }, "enter-send")).toBe(false);

    expect(shouldSelectPopupOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
      isComposing: true,
    }, "enter-send")).toBe(false);
  });

  it("uses Enter for popup selection only in enter-send mode", () => {
    expect(shouldSelectPopupOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    }, "enter-send")).toBe(true);

    expect(shouldSelectPopupOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: true,
      metaKey: false,
    }, "enter-send")).toBe(false);

    expect(shouldSelectPopupOnEnter({
      key: "Enter",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    }, "mod-enter-send")).toBe(false);
  });

  it("returns platform-aware primary modifier labels", () => {
    expect(getChatSubmitModifierLabel("default")).toBe("Ctrl");
    expect(getChatSubmitModifierLabel("mac")).toBe("Cmd");
  });
});
