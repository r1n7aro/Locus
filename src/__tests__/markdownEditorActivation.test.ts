// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  captureMarkdownEditorActivation,
  focusMarkdownEditorAtActivation,
  placeMarkdownEditorCaretAtActivation,
  restoreMarkdownEditorActivationScroll,
} from "../components/ui/markdownEditorActivation";

afterEach(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(document, "caretPositionFromPoint");
  Reflect.deleteProperty(document, "caretRangeFromPoint");
  document.body.replaceChildren();
});

function createEditorFixture() {
  const scroller = document.createElement("div");
  scroller.style.overflowY = "auto";
  scroller.scrollTop = 420;
  scroller.scrollLeft = 24;

  const editor = document.createElement("div");
  const text = document.createTextNode("alpha beta gamma");
  editor.appendChild(text);
  scroller.appendChild(editor);
  document.body.appendChild(scroller);
  vi.spyOn(editor, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 0,
    width: 720,
    height: 980,
    top: 0,
    right: 720,
    bottom: 980,
    left: 0,
    toJSON: () => ({}),
  });

  return { scroller, editor, text };
}

describe("markdown editor rendered activation", () => {
  it("captures and restores scrollable ancestors while retaining rendered height", () => {
    const { scroller, editor } = createEditorFixture();
    const snapshot = captureMarkdownEditorActivation(editor, { x: 180, y: 320 });

    expect(snapshot.point).toEqual({ x: 180, y: 320 });
    expect(snapshot.renderedHeight).toBe(980);

    scroller.scrollTop = 0;
    scroller.scrollLeft = 0;
    restoreMarkdownEditorActivationScroll(snapshot);

    expect(scroller.scrollTop).toBe(420);
    expect(scroller.scrollLeft).toBe(24);
  });

  it("focuses without scrolling and places the caret at the captured click point", () => {
    const { scroller, editor, text } = createEditorFixture();
    const snapshot = captureMarkdownEditorActivation(editor, { x: 180, y: 320 });
    const caretPositionFromPoint = vi.fn(() => {
      expect(scroller.scrollTop).toBe(420);
      return {
        offsetNode: text,
        offset: 6,
        getClientRect: () => null,
      };
    });
    Object.defineProperty(document, "caretPositionFromPoint", {
      configurable: true,
      value: caretPositionFromPoint,
    });
    const focus = vi.spyOn(editor, "focus");
    scroller.scrollTop = 0;

    focusMarkdownEditorAtActivation(editor, snapshot);

    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(caretPositionFromPoint).toHaveBeenCalledWith(180, 320);
    expect(document.getSelection()?.anchorNode).toBe(text);
    expect(document.getSelection()?.anchorOffset).toBe(6);
    expect(scroller.scrollTop).toBe(420);
  });

  it("falls back to caretRangeFromPoint and ignores ranges outside the editor", () => {
    const { editor, text } = createEditorFixture();
    const snapshot = captureMarkdownEditorActivation(editor, { x: 180, y: 320 });
    Object.defineProperty(document, "caretPositionFromPoint", {
      configurable: true,
      value: vi.fn(() => null),
    });
    const insideRange = document.createRange();
    insideRange.setStart(text, 3);
    insideRange.collapse(true);
    const caretRangeFromPoint = vi.fn(() => insideRange);
    Object.defineProperty(document, "caretRangeFromPoint", {
      configurable: true,
      value: caretRangeFromPoint,
    });

    expect(placeMarkdownEditorCaretAtActivation(editor, snapshot)).toBe(true);
    expect(caretRangeFromPoint).toHaveBeenCalledWith(180, 320);
    expect(document.getSelection()?.anchorOffset).toBe(3);

    const outside = document.createElement("div");
    outside.textContent = "outside";
    document.body.appendChild(outside);
    const outsideRange = document.createRange();
    outsideRange.setStart(outside.firstChild!, 2);
    outsideRange.collapse(true);
    caretRangeFromPoint.mockReturnValue(outsideRange);

    expect(placeMarkdownEditorCaretAtActivation(editor, snapshot)).toBe(false);
  });
});
